# Publishing (maintainers)

How the packages get to their registries. This is release plumbing, not something users need — the
user-facing install instructions are in the [README](../README.md#installation) and the
[language guides](languages/README.md).

```sh
nu scripts/publish_packages.nu stage <language>                       # bindings + native libs -> dist/<language>
nu scripts/publish_packages.nu publish <language> <version> --dry-run  # print the plan; --target github|gitea|public
```

## Public registries

The Gitea release workflow publishes to the indexes people install from (`--target public`). Each job needs
its own repository secret and **skips quietly without it**:

| Language | Registry | Package | Secret(s) | Notes |
| --- | --- | --- | --- | --- |
| Rust | crates.io | `stem-mqtt-client`, `stem-mqtt-broker` | `CRATES_IO_TOKEN` | |
| Python | PyPI | `stem-mqtt-client`, `stem-mqtt-broker` | `PYPI_API_TOKEN` | manylinux_2_28 wheels (zig) |
| Node.js | npmjs.org | `stem-mqtt` | `NPM_TOKEN` | granular token with *bypass 2FA*; Linux x86_64 prebuild today |
| C# | NuGet.org | `StemMqtt` | `NUGET_API_KEY` | |
| Kotlin / Java | Maven Central | `io.github.<owner>.stemmqtt:stem-mqtt-kotlin` / `…-java` | `SONATYPE_TOKEN` (the Portal's `settings.xml` snippet) or `MAVEN_CENTRAL_USERNAME` + `MAVEN_CENTRAL_PASSWORD`, `SIGNING_KEY`, `SIGNING_PASSWORD` | verify the `io.github.<owner>` namespace first |
| Haskell | Hackage | `stem-mqtt` | `HACKAGE_TOKEN` | source package, builds the bundled Rust with `cargo` |
| Dart | pub.dev | `stem_mqtt` | — (OIDC) | from GitHub Actions (`publish-pubdev.yml`); first release by hand |
| Go | Go modules | `github.com/<owner>/stem-mqtt-go` | `GO_MODULE_TOKEN` | pushes the module to its own repo and tags it |
| Swift | — | release asset (XCFrameworks) | — | needs macOS |

The Haskell and Dart packages are exactly what their runtime tests exercise. The Node, C#, Java, Kotlin and
Go packages bundle or expect native libraries only for the platforms the release runner builds (Linux x86_64
today); other platforms use the release assets or a source build. Maven Central and Hackage releases cannot
be deleted — read the job logs of the first release.

## Project registries

Besides the public indexes, every language also goes to GitHub Packages (Maven, npm, NuGet, and OCI bundles
on `ghcr.io` for Go, Dart and Haskell) and/or your Gitea instance's own registries
(`--target gitea --base-url http://host:3000`, secret `PACKAGES_TOKEN` with `write:package`). Kotlin's Android AAR
and the Swift XCFrameworks need the GitHub workflow (NDK / macOS). A manual workflow
(`.github/workflows/publish-packages.yml`) dry-runs or publishes a throw-away version such as `0.0.0-rc.1`
without cutting a release. See [`packaging/README.md`](../packaging/README.md) for the artifact layout.
