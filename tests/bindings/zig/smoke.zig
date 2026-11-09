//! Runtime smoke test for the Zig bindings: client + broker, both callback
//! directions, real sockets. Prints "SMOKE OK" on success.
//!
//! Scenario (identical in every language's smoke test): start a broker with an
//! auth provider and an event listener; subscribe with one client; publish QoS 1
//! from another; expect delivery; expect a bad-credentials client to be refused;
//! expect the broker's event listener to have seen the connections; stop.
const std = @import("std");
const client = @import("mqtt_client.zig");
const broker_mod = @import("mqtt_broker.zig");

const gpa = std.heap.c_allocator;

fn must(cond: bool, what: []const u8) void {
    if (!cond) {
        std.debug.print("FAIL {s}\n", .{what});
        std.process.exit(1);
    }
}

fn sleepMs(ms: u32) void {
    var req: std.c.timespec = .{ .sec = 0, .nsec = @as(c_long, ms) * 1_000_000 };
    _ = std.c.nanosleep(&req, null);
}

/// The subscriber's message listener: records whether the expected message arrived.
const Inbox = struct {
    got: std.atomic.Value(bool) = .init(false),
    matches: std.atomic.Value(bool) = .init(false),

    pub fn onMessage(self: *Inbox, allocator: std.mem.Allocator, message: client.MqttMessage) !void {
        _ = allocator;
        self.matches.store(std.mem.eql(u8, message.topic, "smoke/zig") and std.mem.eql(u8, message.payload, "hello-zig"), .release);
        self.got.store(true, .release);
    }

    pub fn onDisconnected(self: *Inbox, allocator: std.mem.Allocator, reason: []const u8) !void {
        _ = self;
        _ = allocator;
        _ = reason;
    }
};

const Auth = struct {
    pub fn authenticate(self: *Auth, allocator: std.mem.Allocator, client_id: []const u8, username: ?[]const u8, password: ?[]const u8) !bool {
        _ = self;
        _ = allocator;
        _ = client_id;
        _ = password;
        const user = username orelse return true;
        return !std.mem.eql(u8, user, "bad");
    }
};

const Events = struct {
    sub: std.atomic.Value(bool) = .init(false),
    publisher: std.atomic.Value(bool) = .init(false),

    pub fn onClientConnected(self: *Events, allocator: std.mem.Allocator, client_id: []const u8) !void {
        _ = allocator;
        if (std.mem.eql(u8, client_id, "zig-sub")) self.sub.store(true, .release);
        if (std.mem.eql(u8, client_id, "zig-pub")) self.publisher.store(true, .release);
    }

    pub fn onClientDisconnected(self: *Events, allocator: std.mem.Allocator, client_id: []const u8, reason: []const u8) !void {
        _ = self;
        _ = allocator;
        _ = client_id;
        _ = reason;
    }

    pub fn onMessagePublished(self: *Events, allocator: std.mem.Allocator, client_id: []const u8, topic: []const u8, qos: broker_mod.QoS) !void {
        _ = self;
        _ = allocator;
        _ = client_id;
        _ = topic;
        _ = qos;
    }
};

fn options(port: u16, id: []const u8, user: ?[]const u8) client.ConnectOptions {
    return .{
        .host = "127.0.0.1",
        .port = port,
        .client_id = id,
        .version = .v5,
        .clean_start = true,
        .keep_alive_secs = 30,
        .username = user,
        .password = null,
        .will = null,
        .connect_timeout_secs = 10,
        .operation_timeout_secs = 5,
        .auto_reconnect = false,
        .reconnect_backoff_secs = 0,
        .reconnect_max_backoff_secs = 0,
        .tls = null,
    };
}

pub fn main() !void {
    const broker = try broker_mod.MqttBroker.init(.{
        .bind_address = "127.0.0.1",
        .port = 0,
        .ws_port = null,
        .allow_anonymous = true,
        .max_clients = 0,
        .max_qos = .exactly_once,
        .max_retained_messages = 100,
        .max_queued_per_client = 100,
        .redelivery_interval_secs = 0,
        .tls = null,
    });
    defer broker.deinit();
    var auth: Auth = .{};
    var events: Events = .{};
    const provider = broker_mod.MqttAuthProvider.fromImpl(&auth);
    defer provider.deinit();
    try broker.setAuthProvider(provider);
    const listener = broker_mod.MqttBrokerEventListener.fromImpl(&events);
    defer listener.deinit();
    try broker.setEventListener(listener);
    try broker.start(gpa, null);
    must(try broker.isRunning(), "broker running");
    const port = (try broker.boundPort()) orelse 0;
    must(port != 0, "bound port");

    var inbox: Inbox = .{};
    const sub = try client.MqttClient.init(options(port, "zig-sub", null));
    defer sub.deinit();
    const inbox_listener = client.MqttMessageListener.fromImpl(&inbox);
    defer inbox_listener.deinit();
    try sub.setMessageListener(inbox_listener);
    _ = try sub.connect(gpa, null);
    const granted = try sub.subscribe(gpa, "smoke/#", .at_least_once, null);
    must(granted.reason_code < 0x80, "subscribe granted");

    const publisher = try client.MqttClient.init(options(port, "zig-pub", null));
    defer publisher.deinit();
    _ = try publisher.connect(gpa, null);
    try publisher.publish(gpa, "smoke/zig", "hello-zig", .at_least_once, false, null);

    var waited: u32 = 0;
    while (!inbox.got.load(.acquire) and waited < 5000) : (waited += 20) sleepMs(20);
    must(inbox.got.load(.acquire), "message delivery timed out");
    must(inbox.matches.load(.acquire), "message content");

    const bad = try client.MqttClient.init(options(port, "zig-bad", "bad"));
    defer bad.deinit();
    if (bad.connect(gpa, null)) |_| {
        must(false, "auth provider must refuse bad credentials");
    } else |_| {}

    must(events.sub.load(.acquire) and events.publisher.load(.acquire), "event listener saw connections");

    try publisher.disconnect(gpa, null);
    try sub.disconnect(gpa, null);
    try broker.stop(gpa, null);
    must(!try broker.isRunning(), "broker stopped");
    const ok = "SMOKE OK\n";
    _ = std.c.write(1, ok, ok.len);
}
