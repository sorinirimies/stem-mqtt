/* Runtime smoke test for the C bindings: client + broker, both callback
 * directions, real sockets. Prints "SMOKE OK" on success.
 *
 * Scenario (identical in every language's smoke test): start a broker with an
 * auth provider and an event listener; subscribe with one client; publish QoS 1
 * from another; expect delivery; expect a bad-credentials client to be refused;
 * expect the broker's event listener to have seen the connections; stop.
 */
#define _POSIX_C_SOURCE 200809L /* clock_gettime, pthreads under -std=c11 on glibc */
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "mqtt_broker.h"
#include "mqtt_client.h"

static void must(bool cond, const char *what) {
    if (!cond) {
        fprintf(stderr, "FAIL %s\n", what);
        exit(1);
    }
}

/* Aborts with the message when a call failed (status code != 0). */
static void check(uniffi_status *status, const char *what) {
    if (status->code != 0) {
        fprintf(stderr, "FAIL %s: %.*s\n", what, (int)status->message.len, status->message.data);
        exit(1);
    }
}

static bool text_is(uniffi_string s, const char *text) {
    return s.len == strlen(text) && memcmp(s.data, text, s.len) == 0;
}

/* ---- The subscriber's message listener: hands one message to main. ---- */

typedef struct {
    pthread_mutex_t lock;
    pthread_cond_t arrived;
    bool got;
    bool matches;
} inbox;

static void on_message(void *ctx, mqtt_client_MqttMessage m, uniffi_status *status) {
    (void)status;
    inbox *in = ctx;
    pthread_mutex_lock(&in->lock);
    in->matches = text_is(m.topic, "smoke/c") && m.payload.len == 7 && memcmp(m.payload.data, "hello-c", 7) == 0;
    in->got = true;
    pthread_cond_signal(&in->arrived);
    pthread_mutex_unlock(&in->lock);
}

static void on_disconnected(void *ctx, uniffi_string reason, uniffi_status *status) {
    (void)ctx, (void)reason, (void)status;
}

static const mqtt_client_MqttMessageListener_vtable INBOX = {
    .on_message = on_message, .on_disconnected = on_disconnected, .uniffi_release = NULL};

/* ---- The broker's auth provider and event listener. ---- */

static bool authenticate(void *ctx, uniffi_string client_id, mqtt_broker_OptionString username,
                         mqtt_broker_OptionBytes password, uniffi_status *status) {
    (void)ctx, (void)client_id, (void)password, (void)status;
    return !username.has_value || !text_is(username.value, "bad");
}

static const mqtt_broker_MqttAuthProvider_vtable AUTH = {.authenticate = authenticate, .uniffi_release = NULL};

typedef struct {
    pthread_mutex_t lock;
    bool sub, pub;
} events;

static void on_client_connected(void *ctx, uniffi_string id, uniffi_status *status) {
    (void)status;
    events *ev = ctx;
    pthread_mutex_lock(&ev->lock);
    ev->sub |= text_is(id, "c-sub");
    ev->pub |= text_is(id, "c-pub");
    pthread_mutex_unlock(&ev->lock);
}

static void on_client_disconnected(void *ctx, uniffi_string id, uniffi_string reason, uniffi_status *status) {
    (void)ctx, (void)id, (void)reason, (void)status;
}

static void on_message_published(void *ctx, uniffi_string id, uniffi_string topic, mqtt_broker_QoS qos,
                                 uniffi_status *status) {
    (void)ctx, (void)id, (void)topic, (void)qos, (void)status;
}

static const mqtt_broker_MqttBrokerEventListener_vtable EVENTS = {.on_client_connected = on_client_connected,
                                                                  .on_client_disconnected = on_client_disconnected,
                                                                  .on_message_published = on_message_published,
                                                                  .uniffi_release = NULL};

static mqtt_client_ConnectOptions options(uint16_t port, const char *id, const char *user) {
    mqtt_client_ConnectOptions o = mqtt_client_ConnectOptions_DEFAULTS;
    o.host = uniffi_str("127.0.0.1");
    o.port = port;
    o.client_id = uniffi_str(id);
    o.version = mqtt_client_MqttVersion_V5;
    o.clean_start = true;
    o.keep_alive_secs = 30;
    if (user != NULL) {
        o.username.has_value = true;
        o.username.value = uniffi_str(user);
    }
    o.connect_timeout_secs = 10;
    o.operation_timeout_secs = 5;
    return o;
}

int main(void) {
    uniffi_status st = {0};

    mqtt_broker_MqttBrokerConfig config = mqtt_broker_MqttBrokerConfig_DEFAULTS;
    config.bind_address = uniffi_str("127.0.0.1");
    config.port = 0;
    config.allow_anonymous = true;
    config.max_qos = mqtt_broker_QoS_ExactlyOnce;
    config.max_retained_messages = 100;
    config.max_queued_per_client = 100;
    mqtt_broker_MqttBroker broker = mqtt_broker_MqttBroker_new(config, &st);
    check(&st, "broker new");

    events ev = {.lock = PTHREAD_MUTEX_INITIALIZER};
    mqtt_broker_MqttAuthProvider auth = mqtt_broker_MqttAuthProvider_from_impl(&AUTH, NULL);
    mqtt_broker_MqttBroker_set_auth_provider(broker, auth, &st);
    check(&st, "set auth provider");
    mqtt_broker_MqttAuthProvider_free(auth);
    mqtt_broker_MqttBrokerEventListener listener = mqtt_broker_MqttBrokerEventListener_from_impl(&EVENTS, &ev);
    mqtt_broker_MqttBroker_set_event_listener(broker, listener, &st);
    check(&st, "set event listener");
    mqtt_broker_MqttBrokerEventListener_free(listener);

    mqtt_broker_MqttBrokerError berr = {0};
    mqtt_broker_MqttBroker_start(broker, &berr, &st);
    check(&st, "broker start");
    must(mqtt_broker_MqttBroker_is_running(broker, &st), "broker running");
    mqtt_broker_OptionU16 bound = mqtt_broker_MqttBroker_bound_port(broker, &st);
    must(bound.has_value && bound.value != 0, "bound port");
    uint16_t port = bound.value;

    inbox in = {.lock = PTHREAD_MUTEX_INITIALIZER, .arrived = PTHREAD_COND_INITIALIZER};
    mqtt_client_MqttClient sub = mqtt_client_MqttClient_new(options(port, "c-sub", NULL), &st);
    check(&st, "sub new");
    mqtt_client_MqttMessageListener inbox_listener = mqtt_client_MqttMessageListener_from_impl(&INBOX, &in);
    mqtt_client_MqttClient_set_message_listener(sub, inbox_listener, &st);
    check(&st, "set message listener");
    mqtt_client_MqttMessageListener_free(inbox_listener);
    mqtt_client_MqttError err = {0};
    mqtt_client_MqttClient_connect(sub, &err, &st);
    check(&st, "sub connect");
    mqtt_client_SubscribeResult granted =
        mqtt_client_MqttClient_subscribe(sub, uniffi_str("smoke/#"), mqtt_client_QoS_AtLeastOnce, &err, &st);
    check(&st, "subscribe");
    must(granted.reason_code < 0x80, "subscribe granted");

    mqtt_client_MqttClient pub = mqtt_client_MqttClient_new(options(port, "c-pub", NULL), &st);
    check(&st, "pub new");
    mqtt_client_MqttClient_connect(pub, &err, &st);
    check(&st, "pub connect");
    mqtt_client_MqttClient_publish(pub, uniffi_str("smoke/c"), uniffi_bytes_view("hello-c", 7),
                                   mqtt_client_QoS_AtLeastOnce, false, &err, &st);
    check(&st, "publish");

    struct timespec deadline;
    clock_gettime(CLOCK_REALTIME, &deadline);
    deadline.tv_sec += 5;
    pthread_mutex_lock(&in.lock);
    while (!in.got) {
        if (pthread_cond_timedwait(&in.arrived, &in.lock, &deadline) != 0) break;
    }
    must(in.got, "message delivery timed out");
    must(in.matches, "message content");
    pthread_mutex_unlock(&in.lock);

    mqtt_client_MqttClient bad = mqtt_client_MqttClient_new(options(port, "c-bad", "bad"), &st);
    check(&st, "bad new");
    mqtt_client_MqttClient_connect(bad, &err, &st);
    must(st.code != 0, "auth provider must refuse bad credentials");
    mqtt_client_MqttError_free(&err);
    uniffi_status_free(&st);
    mqtt_client_MqttClient_free(bad);

    pthread_mutex_lock(&ev.lock);
    must(ev.sub && ev.pub, "event listener saw connections");
    pthread_mutex_unlock(&ev.lock);

    mqtt_client_MqttClient_disconnect(pub, &err, &st);
    check(&st, "pub disconnect");
    mqtt_client_MqttClient_disconnect(sub, &err, &st);
    check(&st, "sub disconnect");
    mqtt_client_MqttClient_free(pub);
    mqtt_client_MqttClient_free(sub);
    mqtt_broker_MqttBroker_stop(broker, &berr, &st);
    check(&st, "broker stop");
    must(!mqtt_broker_MqttBroker_is_running(broker, &st), "broker stopped");
    mqtt_broker_MqttBroker_free(broker);
    puts("SMOKE OK");
    return 0;
}
