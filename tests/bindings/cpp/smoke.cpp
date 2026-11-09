// Runtime smoke test for the C++ bindings: client + broker, both callback
// directions, real sockets. Prints "SMOKE OK" on success.
//
// Scenario (identical in every language's smoke test): start a broker with an
// auth provider and an event listener; subscribe with one client; publish QoS 1
// from another; expect delivery; expect a bad-credentials client to be refused;
// expect the broker's event listener to have seen the connections; stop.
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <memory>
#include <mutex>
#include <optional>
#include <set>
#include <string>
#include <vector>

#include "mqtt_broker.hpp"
#include "mqtt_client.hpp"

namespace {

void must(bool cond, const char* what) {
    if (!cond) {
        std::fprintf(stderr, "FAIL %s\n", what);
        std::exit(1);
    }
}

class Inbox final : public mqtt_client::MqttMessageListener {
public:
    void on_message(const mqtt_client::MqttMessage& message) override {
        std::lock_guard<std::mutex> lock(mutex_);
        messages_.push_back(message);
        arrived_.notify_all();
    }
    void on_disconnected(const std::string&) override {}

    std::optional<mqtt_client::MqttMessage> wait(std::chrono::seconds timeout) {
        std::unique_lock<std::mutex> lock(mutex_);
        if (!arrived_.wait_for(lock, timeout, [&] { return !messages_.empty(); })) return std::nullopt;
        return messages_.front();
    }

private:
    std::mutex mutex_;
    std::condition_variable arrived_;
    std::vector<mqtt_client::MqttMessage> messages_;
};

class Auth final : public mqtt_broker::MqttAuthProvider {
public:
    bool authenticate(const std::string&, const std::optional<std::string>& username,
                      const std::optional<std::vector<uint8_t>>&) override {
        return !username || *username != "bad";
    }
};

class Events final : public mqtt_broker::MqttBrokerEventListener {
public:
    void on_client_connected(const std::string& client_id) override {
        std::lock_guard<std::mutex> lock(mutex_);
        connected_.insert(client_id);
    }
    void on_client_disconnected(const std::string&, const std::string&) override {}
    void on_message_published(const std::string&, const std::string&, mqtt_broker::QoS) override {}

    bool saw(const std::string& client_id) {
        std::lock_guard<std::mutex> lock(mutex_);
        return connected_.count(client_id) != 0;
    }

private:
    std::mutex mutex_;
    std::set<std::string> connected_;
};

mqtt_client::ConnectOptions options(uint16_t port, const std::string& id, std::optional<std::string> user) {
    mqtt_client::ConnectOptions o;
    o.host = "127.0.0.1";
    o.port = port;
    o.client_id = id;
    o.version = mqtt_client::MqttVersion::V5;
    o.clean_start = true;
    o.keep_alive_secs = 30;
    o.username = std::move(user);
    o.connect_timeout_secs = 10;
    o.operation_timeout_secs = 5;
    return o;
}

}  // namespace

int main() {
    try {
        mqtt_broker::MqttBrokerConfig config;
        config.bind_address = "127.0.0.1";
        config.port = 0;
        config.allow_anonymous = true;
        config.max_qos = mqtt_broker::QoS::ExactlyOnce;
        config.max_retained_messages = 100;
        config.max_queued_per_client = 100;
        auto broker = mqtt_broker::MqttBroker::new_(config);
        auto events = std::make_shared<Events>();
        broker->set_auth_provider(std::make_shared<Auth>());
        broker->set_event_listener(events);
        broker->start().get();
        must(broker->is_running(), "broker running");
        auto port = broker->bound_port();
        must(port && *port != 0, "bound port");

        auto inbox = std::make_shared<Inbox>();
        auto sub = mqtt_client::MqttClient::new_(options(*port, "cpp-sub", std::nullopt));
        sub->set_message_listener(inbox);
        sub->connect().get();
        must(sub->subscribe("smoke/#", mqtt_client::QoS::AtLeastOnce).get().reason_code < 0x80, "subscribe granted");

        auto pub = mqtt_client::MqttClient::new_(options(*port, "cpp-pub", std::nullopt));
        pub->connect().get();
        const std::string payload = "hello-cpp";
        pub->publish("smoke/cpp", std::vector<uint8_t>(payload.begin(), payload.end()), mqtt_client::QoS::AtLeastOnce,
                     false)
            .get();

        auto message = inbox->wait(std::chrono::seconds(5));
        must(message.has_value(), "message delivery timed out");
        must(message->topic == "smoke/cpp" &&
                 message->payload == std::vector<uint8_t>(payload.begin(), payload.end()),
             "message content");

        bool refused = false;
        try {
            mqtt_client::MqttClient::new_(options(*port, "cpp-bad", std::string("bad")))->connect().get();
        } catch (const mqtt_client::MqttError&) {
            refused = true;
        }
        must(refused, "auth provider must refuse bad credentials");

        must(events->saw("cpp-sub") && events->saw("cpp-pub"), "event listener saw connections");

        pub->disconnect().get();
        sub->disconnect().get();
        broker->stop().get();
        must(!broker->is_running(), "broker stopped");
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL %s\n", e.what());
        return 1;
    }
    std::puts("SMOKE OK");
    return 0;
}
