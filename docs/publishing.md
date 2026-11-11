# Publishing (maintainers)

How the packages get to their registries. This is release plumbing, not something users need — the
user-facing install instructions are in the [README](../README.md#installation) and the
[language guides](languages/README.md).

```sh
nu scripts/publish_packages.nu stage <language>                       # bindings + native libs -> dist/<language>
nu scripts/publish_packages.nu publish <language> <version> --dry-run  # print the plan; --target github|gitea|public
```

## Public registries

The release workflows publish to the indexes people install from (`--target public`); a version the other
host published first counts as done. npm and Maven Central go out from GitHub only: their packages are one per
platform (`--natives`), and only GitHub's release builds every platform's libraries (`native-libs.yml`).
Each job needs its own repository secret and **skips quietly without it**:

| Language | Registry | Package | Secret(s) | Notes |
| --- | --- | --- | --- | --- |
| Rust | crates.io | `stem-mqtt-proto`, `stem-mqtt-client`, `stem-mqtt-broker` | `CRATES_IO_TOKEN` | in that order: the client and the broker depend on the crate before them (`just publish`) |
| Python | PyPI | `stem-mqtt-client`, `stem-mqtt-broker` | `PYPI_API_TOKEN` | manylinux_2_28 wheels (zig) |
| Node.js | npmjs.org | `stem-mqtt` | `NPM_TOKEN` | granular token with *bypass 2FA*; plus `stem-mqtt-<platform>` packages, published first |
| C# | NuGet.org | `StemMqtt` | repository variable `NUGET_USER` (trusted publishing), or `NUGET_API_KEY` | trusted publishing: nuget.org → account → Trusted Publishing: owner, repository `stem-mqtt`, workflow `release.yml` |
| Kotlin / Java | Maven Central | `io.github.<owner>.stemmqtt:stem-mqtt-kotlin` / `…-java` | `SONATYPE_TOKEN` (the Portal's `settings.xml` snippet, pasted as is; set on both GitHub and Gitea) or `MAVEN_CENTRAL_USERNAME` + `MAVEN_CENTRAL_PASSWORD`, `SIGNING_KEY`, `SIGNING_PASSWORD` | verify the `io.github.<owner>` namespace first |
| Haskell | Hackage | `stem-mqtt` | `HACKAGE_TOKEN` | source package, builds the bundled Rust with `cargo` |
| Dart | pub.dev | `stem_mqtt` | — (OIDC) | from GitHub Actions (`publish-pubdev.yml`); first release by hand |
| Go | Go modules | `github.com/<owner>/stem-mqtt/go` | none (`GITHUB_TOKEN`) | tags `go/v<version>` on a commit holding only the generated module; no separate repository |
| Swift | — | release asset (XCFrameworks) | — | needs macOS |

The Haskell and Dart packages are exactly what their runtime tests exercise. npm and Maven carry every
platform the release builds (Linux x86_64 and aarch64, macOS x86_64 and arm64, Windows x86_64), one package or
classifier jar each. The C#, Kotlin and Go packages bundle or expect native libraries only for Linux x86_64;
other platforms use the release assets or a source build. `publish-npm.yml` publishes npm without a release. Maven Central and Hackage releases cannot
be deleted — read the job logs of the first release.

## Project registries

Besides the public indexes, every language also goes to GitHub Packages (Maven, npm, NuGet, and OCI bundles
on `ghcr.io` for Go, Dart and Haskell) and/or your Gitea instance's own registries
(`--target gitea --base-url http://host:3000`, secret `PACKAGES_TOKEN` with `write:package`). Kotlin's Android AAR
and the Swift XCFrameworks need the GitHub workflow (NDK / macOS). A manual workflow
(`.github/workflows/publish-packages.yml`) dry-runs or publishes a throw-away version such as `0.0.0-rc.1`
without cutting a release. See [`packaging/README.md`](../packaging/README.md) for the artifact layout.
