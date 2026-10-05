// Runtime smoke test for the Go bindings: client + broker, both callback
// directions, real sockets. Prints "SMOKE OK" on success.
//
// Scenario (identical in every language's smoke test): start a broker with an
// auth provider and an event listener; subscribe with one client; publish QoS 1
// from another; expect delivery; expect a bad-credentials client to be refused;
// expect the broker's event listener to have seen the connections; stop.
package main

import (
	"bytes"
	"fmt"
	"os"
	"sync"
	"time"

	"mqtt_broker"
	"mqtt_client"
)

type inbox struct{ ch chan mqtt_client.MqttMessage }

func (i *inbox) OnMessage(m mqtt_client.MqttMessage) { i.ch <- m }
func (i *inbox) OnDisconnected(reason string)         {}

type auth struct{}

func (auth) Authenticate(clientId string, username *string, password *[]byte) bool {
	return username == nil || *username != "bad"
}

type events struct {
	mu        sync.Mutex
	connected map[string]bool
}

func (e *events) OnClientConnected(id string) {
	e.mu.Lock()
	e.connected[id] = true
	e.mu.Unlock()
}
func (e *events) OnClientDisconnected(id, reason string)                           {}
func (e *events) OnMessagePublished(id, topic string, qos mqtt_broker.QoS) {}

func options(port uint16, id string, user *string) mqtt_client.ConnectOptions {
	return mqtt_client.ConnectOptions{
		Host: "127.0.0.1", Port: port, ClientId: id,
		Version: mqtt_client.MqttVersionV5, CleanStart: true, KeepAliveSecs: 30,
		Username: user, ConnectTimeoutSecs: 10, OperationTimeoutSecs: 5,
	}
}

func check(err error, what string) {
	if err != nil {
		fmt.Fprintf(os.Stderr, "FAIL %s: %v\n", what, err)
		os.Exit(1)
	}
}

func must(cond bool, what string) {
	if !cond {
		fmt.Fprintf(os.Stderr, "FAIL %s\n", what)
		os.Exit(1)
	}
}

func main() {
	broker := mqtt_broker.NewMqttBroker(mqtt_broker.MqttBrokerConfig{
		BindAddress: "127.0.0.1", Port: 0, AllowAnonymous: true,
		MaxQos: mqtt_broker.QoSExactlyOnce, MaxRetainedMessages: 100, MaxQueuedPerClient: 100,
	})
	ev := &events{connected: map[string]bool{}}
	broker.SetAuthProvider(auth{})
	broker.SetEventListener(ev)
	check(broker.Start(), "broker start")
	must(broker.IsRunning(), "broker running")
	portPtr := broker.BoundPort()
	must(portPtr != nil && *portPtr != 0, "bound port")
	port := *portPtr

	in := &inbox{ch: make(chan mqtt_client.MqttMessage, 4)}
	sub := mqtt_client.NewMqttClient(options(port, "go-sub", nil))
	sub.SetMessageListener(in)
	_, err := sub.Connect()
	check(err, "sub connect")
	res, err := sub.Subscribe("smoke/#", mqtt_client.QoSAtLeastOnce)
	check(err, "subscribe")
	must(res.ReasonCode < 0x80, "subscribe granted")

	pub := mqtt_client.NewMqttClient(options(port, "go-pub", nil))
	_, err = pub.Connect()
	check(err, "pub connect")
	check(pub.Publish("smoke/go", []byte("hello-go"), mqtt_client.QoSAtLeastOnce, false), "publish")

	select {
	case m := <-in.ch:
		must(m.Topic == "smoke/go" && bytes.Equal(m.Payload, []byte("hello-go")), "message content")
	case <-time.After(5 * time.Second):
		must(false, "message delivery timed out")
	}

	bad := "bad"
	_, err = mqtt_client.NewMqttClient(options(port, "go-bad", &bad)).Connect()
	must(err != nil, "auth provider must refuse bad credentials")

	ev.mu.Lock()
	must(ev.connected["go-sub"] && ev.connected["go-pub"], "event listener saw connections")
	ev.mu.Unlock()

	check(pub.Disconnect(), "pub disconnect")
	check(sub.Disconnect(), "sub disconnect")
	check(broker.Stop(), "broker stop")
	must(!broker.IsRunning(), "broker stopped")
	fmt.Println("SMOKE OK")
}
