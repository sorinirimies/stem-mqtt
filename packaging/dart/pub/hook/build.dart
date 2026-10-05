// Native-assets build hook: compiles the bundled Rust core (rust/) with cargo and registers the
// resulting library under the asset id the generated bindings load
// (`package:stem_mqtt/uniffi:mqtt_broker`; the client component lives in the same library).
//
// Requires a Rust toolchain (https://rustup.rs). The library is built for the machine running
// the hook (Dart VM / `dart run` / `dart compile exe`); cross-compiling for Flutter mobile
// targets is not supported yet.
import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:hooks/hooks.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) return;

    final os = input.config.code.targetOS;
    final hostOs = OS.current;
    if (os != hostOs) {
      throw UnsupportedError(
          'stem_mqtt builds its Rust core on the host only (target $os, host $hostOs).');
    }

    final rustDir = input.packageRoot.resolve('rust/');
    final targetDir = input.outputDirectory.resolve('cargo-target/');
    final result = await Process.run(
      'cargo',
      [
        'build',
        '--release',
        '--locked',
        '-p',
        'stem-mqtt-broker',
        '--manifest-path',
        rustDir.resolve('Cargo.toml').toFilePath(),
      ],
      environment: {'CARGO_TARGET_DIR': targetDir.toFilePath()},
    );
    if (result.exitCode != 0) {
      throw StateError('`cargo build` failed - is a Rust toolchain installed '
          '(https://rustup.rs)?\n${result.stdout}\n${result.stderr}');
    }

    final libName = os.dylibFileName('mqtt_broker');
    final lib = targetDir.resolve('release/$libName');
    if (!File.fromUri(lib).existsSync()) {
      throw StateError('cargo did not produce $libName');
    }
    output.assets.code.add(CodeAsset(
      package: input.packageName,
      name: 'uniffi:mqtt_broker',
      linkMode: DynamicLoadingBundled(),
      file: lib,
    ));
    // Re-run when the Rust sources change.
    output.dependencies.add(rustDir.resolve('crates/'));
  });
}
