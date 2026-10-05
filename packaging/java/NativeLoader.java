package io.github.stemmqtt;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

/**
 * Finds the Rust library bundled in this jar.
 *
 * <p>The generated bindings call {@code System.loadLibrary("mqtt_client")}, which only searches
 * {@code java.library.path}. The published jar carries the native libraries as resources instead
 * ({@code /<os>-<arch>/lib<name>.<ext>}), so before a component loads, this class extracts the
 * matching one to a temporary file and points the generated loader at it through the
 * {@code uniffi.component.<name>.libraryOverride} system property.
 *
 * <p>If the property is already set, or the jar has no library for this platform, nothing happens and
 * the normal {@code java.library.path} lookup applies, so a custom build keeps working.
 */
public final class NativeLoader {
    private NativeLoader() {}

    /** Called from the generated {@code NamespaceLibrary.findLibraryName}; safe to call repeatedly. */
    public static synchronized void prepare(String component) {
        String property = "uniffi.component." + component + ".libraryOverride";
        if (System.getProperty(property) != null) {
            return;
        }
        String file = System.mapLibraryName(component);
        String resource = "/" + platform() + "/" + file;
        try (InputStream in = NativeLoader.class.getResourceAsStream(resource)) {
            if (in == null) {
                return; // not bundled for this platform: fall back to java.library.path
            }
            Path dir = Files.createTempDirectory("stem-mqtt-native");
            Path target = dir.resolve(file);
            Files.copy(in, target, StandardCopyOption.REPLACE_EXISTING);
            target.toFile().deleteOnExit();
            dir.toFile().deleteOnExit();
            System.setProperty(property, target.toAbsolutePath().toString());
        } catch (IOException e) {
            throw new UncheckedIOException(component, e);
        }
    }

    /** {@code linux-x86_64}, {@code macos-aarch64}, {@code windows-x86_64}, … */
    static String platform() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);
        String osName = os.contains("win") ? "windows" : os.contains("mac") ? "macos" : "linux";
        String archName = arch.equals("amd64") || arch.equals("x86_64") ? "x86_64"
                : arch.equals("aarch64") || arch.equals("arm64") ? "aarch64" : arch;
        return osName + "-" + archName;
    }

    private static final class UncheckedIOException extends RuntimeException {
        UncheckedIOException(String component, IOException cause) {
            super("could not extract the bundled " + component + " library", cause);
        }
    }
}
