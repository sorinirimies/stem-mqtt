// stem-mqtt browser demo — connects directly to the mqtt-broker's
// WebSocket listener (--ws-port) using mqtt.js, exactly the transport
// path validated by mqtt-broker's `websocket_*` integration tests and the
// mqtt-client-node smoke test.

(() => {
  "use strict";

  const $ = (id) => document.getElementById(id);

  const els = {
    status: $("status"),
    wsUrl: $("ws-url"),
    clientId: $("client-id"),
    protocolVersion: $("protocol-version"),
    btnConnect: $("btn-connect"),
    btnDisconnect: $("btn-disconnect"),
    pubTopic: $("pub-topic"),
    pubPayload: $("pub-payload"),
    pubQos: $("pub-qos"),
    pubRetain: $("pub-retain"),
    btnPublish: $("btn-publish"),
    btnPublishAllQos: $("btn-publish-all-qos"),
    subTopic: $("sub-topic"),
    subQos: $("sub-qos"),
    btnSubscribe: $("btn-subscribe"),
    subscriptionList: $("subscription-list"),
    log: $("log"),
    btnClearLog: $("btn-clear-log"),
  };

  els.clientId.value = "browser-" + Math.random().toString(16).slice(2, 10);

  /** @type {import("mqtt").MqttClient | null} */
  let client = null;
  const subscriptions = new Map(); // topic filter -> qos

  // ── QoS toggle buttons (shared behaviour for publish + subscribe) ──────
  function wireQosToggle(container) {
    container.addEventListener("click", (e) => {
      const btn = e.target.closest(".qos-toggle__option");
      if (!btn) return;
      container.dataset.value = btn.dataset.qos;
      container
        .querySelectorAll(".qos-toggle__option")
        .forEach((b) => b.classList.toggle("is-selected", b === btn));
    });
  }
  wireQosToggle(els.pubQos);
  wireQosToggle(els.subQos);

  // ── Activity log ────────────────────────────────────────────────────────
  function log(kind, text) {
    const entry = document.createElement("div");
    entry.className = `log-entry log-entry--${kind}`;
    const time = new Date().toLocaleTimeString(undefined, { hour12: false });
    entry.innerHTML =
      `<span class="log-entry__time">${time}</span>` +
      `<span class="log-entry__kind">${kind}</span>` +
      `<span class="log-entry__body"></span>`;
    entry.querySelector(".log-entry__body").textContent = text;
    els.log.appendChild(entry);
    els.log.scrollTop = els.log.scrollHeight;
  }
  els.btnClearLog.addEventListener("click", () => (els.log.innerHTML = ""));

  // ── Connection state ────────────────────────────────────────────────────
  function setStatus(state, text) {
    els.status.className = `status-badge status-badge--${state}`;
    els.status.textContent = text;
  }

  function setConnected(connected) {
    els.btnConnect.disabled = connected;
    els.btnDisconnect.disabled = !connected;
    els.btnPublish.disabled = !connected;
    els.btnPublishAllQos.disabled = !connected;
    els.btnSubscribe.disabled = !connected;
  }

  els.btnConnect.addEventListener("click", () => {
    if (client) return;
    const url = els.wsUrl.value.trim();
    const options = {
      clientId: els.clientId.value.trim() || undefined,
      protocolVersion: Number(els.protocolVersion.value),
      clean: true,
      reconnectPeriod: 0, // this demo drives reconnects explicitly
    };

    setStatus("connecting", "connecting…");
    log("system", `connecting to ${url} (MQTT ${options.protocolVersion === 5 ? "5.0" : "3.1.1"})`);

    client = mqtt.connect(url, options);

    client.on("connect", (connack) => {
      setStatus("connected", "connected");
      setConnected(true);
      log("system", `connected (sessionPresent=${connack.sessionPresent}, reasonCode=${connack.reasonCode ?? 0})`);
    });

    client.on("reconnect", () => log("system", "reconnecting…"));

    client.on("close", () => {
      setStatus("disconnected", "disconnected");
      setConnected(false);
      log("system", "connection closed");
    });

    client.on("error", (err) => {
      log("error", err.message || String(err));
    });

    client.on("message", (topic, payload, packet) => {
      const text = tryUtf8(payload);
      log(
        "in",
        `topic=${topic} qos=${packet.qos} retain=${packet.retain} bytes=${payload.length} payload=${text}`,
      );
    });
  });

  els.btnDisconnect.addEventListener("click", () => {
    if (!client) return;
    client.end(true, {}, () => {
      log("system", "disconnected");
      setStatus("disconnected", "disconnected");
      setConnected(false);
      subscriptions.clear();
      renderSubscriptions();
      client = null;
    });
  });

  // ── Publish ──────────────────────────────────────────────────────────────
  function publishOnce(topic, payload, qos, retain) {
    return new Promise((resolve, reject) => {
      const startedAt = performance.now();
      client.publish(topic, payload, { qos, retain }, (err) => {
        const elapsedMs = (performance.now() - startedAt).toFixed(1);
        if (err) {
          reject(err);
          return;
        }
        resolve(elapsedMs);
      });
    });
  }

  els.btnPublish.addEventListener("click", async () => {
    const topic = els.pubTopic.value.trim();
    const payload = els.pubPayload.value;
    const qos = Number(els.pubQos.dataset.value);
    const retain = els.pubRetain.checked;
    if (!topic) return;
    try {
      const elapsedMs = await publishOnce(topic, payload, qos, retain);
      log("out", `published qos=${qos} retain=${retain} topic=${topic} (acked in ${elapsedMs}ms) payload=${payload}`);
    } catch (err) {
      log("error", `publish failed: ${err.message || err}`);
    }
  });

  els.btnPublishAllQos.addEventListener("click", async () => {
    const topic = els.pubTopic.value.trim();
    const payload = els.pubPayload.value;
    const retain = els.pubRetain.checked;
    if (!topic) return;
    log("system", `publishing "${payload}" to ${topic} at QoS 0, 1, and 2 — compare the ack latency below`);
    for (const qos of [0, 1, 2]) {
      try {
        const elapsedMs = await publishOnce(topic, payload, qos, retain);
        log("out", `  QoS ${qos}: acked in ${elapsedMs}ms`);
      } catch (err) {
        log("error", `  QoS ${qos} failed: ${err.message || err}`);
      }
    }
  });

  // ── Subscribe ────────────────────────────────────────────────────────────
  function renderSubscriptions() {
    els.subscriptionList.innerHTML = "";
    for (const [filter, qos] of subscriptions) {
      const li = document.createElement("li");
      li.className = "topcoat-list__item";
      const label = document.createElement("span");
      label.textContent = `${filter} (QoS ${qos})`;
      const btn = document.createElement("button");
      btn.className = "topcoat-button unsubscribe-btn";
      btn.textContent = "Unsubscribe";
      btn.addEventListener("click", () => unsubscribe(filter));
      li.appendChild(label);
      li.appendChild(btn);
      els.subscriptionList.appendChild(li);
    }
  }

  function unsubscribe(filter) {
    if (!client) return;
    client.unsubscribe(filter, (err) => {
      if (err) {
        log("error", `unsubscribe failed: ${err.message || err}`);
        return;
      }
      subscriptions.delete(filter);
      renderSubscriptions();
      log("system", `unsubscribed from ${filter}`);
    });
  }

  els.btnSubscribe.addEventListener("click", () => {
    if (!client) return;
    const filter = els.subTopic.value.trim();
    const qos = Number(els.subQos.dataset.value);
    if (!filter) return;
    client.subscribe(filter, { qos }, (err, granted) => {
      if (err) {
        log("error", `subscribe failed: ${err.message || err}`);
        return;
      }
      const grantedQos = granted?.[0]?.qos ?? qos;
      subscriptions.set(filter, grantedQos);
      renderSubscriptions();
      log("system", `subscribed to ${filter} (granted QoS ${grantedQos})`);
    });
  });

  function tryUtf8(buf) {
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(buf);
    } catch {
      return `<binary ${buf.length} bytes>`;
    }
  }
})();
