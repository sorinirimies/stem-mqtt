# Python packaging (PyPI via maturin)

Both crates ship a `pyproject.toml` (`crates/mqtt-client/pyproject.toml`,
`crates/mqtt-broker/pyproject.toml`) that uses
[maturin](https://www.maturin.rs/)'s built-in UniFFI support
(`bindings = "uniffi"`) — maturin builds the `cdylib`, runs `uniffi-bindgen`
against it, and packages the generated Python wrapper + native library into
one wheel. There's no separate manual bindgen step for Python.

## Building a wheel locally

```sh
pip install maturin
cd crates/mqtt-client   # or crates/mqtt-broker
maturin build --release
# wheel lands in target/wheels/
```

## Publishing (CI)

`.github/workflows/release.yml` first builds both crates' wheels on Linux,
macOS, and Windows and gates GitHub Release creation on those builds. Its
`publish-python` job uploads those exact verified wheels to PyPI when the
`PYPI_API_TOKEN` repository secret is configured:

```sh
gh secret set PYPI_API_TOKEN --body "pypi-AgEI..."
```

If the secret isn't set, registry upload skips, but wheel construction and a
real client-to-broker runtime smoke test are still required on Linux, macOS,
and Windows before GitHub Release creation.

## Distribution vs. import name

The PyPI distribution names are `stem-mqtt-client` / `stem-mqtt-broker`
(avoids clashing with an unrelated `mqtt-client` package that might exist on
PyPI), but the importable module names stay `mqtt_client` / `mqtt_broker`:

```python
import mqtt_client
import mqtt_broker

client = mqtt_client.MqttClient(...)
broker_config = mqtt_broker.MqttBrokerConfig(
    # Broker records use the broker wheel's generated external enum copy.
    max_qos=mqtt_broker.QoS.EXACTLY_ONCE,
    ...
)
```
