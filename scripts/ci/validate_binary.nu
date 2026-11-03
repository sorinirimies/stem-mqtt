#!/usr/bin/env nu
# validate_binary.nu — reject an empty or corrupt release binary before it is
# packaged: it must be a real executable for its target (PE, Mach-O or ELF).
#
# Usage:
#   nu scripts/ci/validate_binary.nu <path> <target>

const MIN_SIZE = 1024

# The executable format a target needs, or null for an unknown target.
export def format_for [target: string]: nothing -> any {
    if ($target =~ "windows") { "PE/COFF (MZ)" } else if ($target =~ "apple") { "Mach-O" } else if ($target =~ "linux") { "ELF" } else { null }
}

# Whether `data` starts like an executable of `format`.
export def has_magic [data: binary, format: string]: nothing -> bool {
    let head = $data | bytes at 0..<4 | encode hex
    match $format {
        "PE/COFF (MZ)" => ($head | str starts-with "4D5A")
        # Mach-O 64 (either byte order) and universal/fat (either byte order).
        "Mach-O" => ($head in ["CFFAEDFE" "FEEDFACF" "CAFEBABE" "BEBAFECA"])
        "ELF" => ($head == "7F454C46")
        _ => false
    }
}

# null when `data` is a plausible executable for `target`, else why it is not.
export def problem [data: binary, target: string]: nothing -> any {
    let format = format_for $target
    if $format == null { return $"unsupported validation target: ($target)" }
    if ($data | bytes length) < $MIN_SIZE { return "missing or implausibly small binary" }
    if ($data | bytes remove -a 0x[00] | bytes length) == 0 { return "binary contains only zero bytes" }
    if not (has_magic $data $format) {
        return $"not ($format); magic=($data | bytes at 0..<16 | encode hex | str lowercase) size=($data | bytes length)"
    }
    null
}

def main [path: string, target: string] {
    let data = if ($path | path exists) { open --raw $path | into binary } else { 0x[] }
    let why = problem $data $target
    if $why != null {
        print -e $"error: ($path): ($why)"
        exit 1
    }
    print -e $"validated (format_for $target): ($path) \(($data | bytes length) bytes)"
}
