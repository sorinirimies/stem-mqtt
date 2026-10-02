// Native-assets build hook: registers the Rust cdylib under the asset id the
// generated bindings expect (`package:uniffi/uniffi:mqtt_broker`). The runner
// copies the library into native/ (build hooks run sandboxed, with no custom
// environment variables, so it is located relative to the package root).
import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:hooks/hooks.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) return;
    final libs = Directory.fromUri(input.packageRoot.resolve('native/'))
        .listSync()
        .whereType<File>()
        .where((f) => f.path.contains('mqtt_broker'))
        .toList();
    if (libs.isEmpty) throw StateError('no mqtt_broker library in native/');
    output.assets.code.add(CodeAsset(
      package: input.packageName,
      name: 'uniffi:mqtt_broker',
      linkMode: DynamicLoadingBundled(),
      file: libs.first.uri,
    ));
  });
}
