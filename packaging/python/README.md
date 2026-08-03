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

`.github/workflows/release.yml`'s `publish-python` job runs
`maturin publish` for both crates on every `vX.Y.Z` tag, gated on the
`PYPI_API_TOKEN` repository secret:

```sh
gh secret set PYPI_API_TOKEN --body "pypi-AgEI..."
```

If the secret isn't set, the job skips with a message instead of failing —
same pattern as the crates.io publish job.

## Distribution vs. import name

The PyPI distribution names are `stem-mqtt-client` / `stem-mqtt-broker`
(avoids clashing with an unrelated `mqtt-client` package that might exist on
PyPI), but the importable module names stay `mqtt_client` / `mqtt_broker`:

```python
import mqtt_client
client = mqtt_client.MqttClient(...)
```
