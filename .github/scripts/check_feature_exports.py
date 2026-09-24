"""Fail the build if a contract's release WASM is missing a function that its
default Cargo features are expected to compile in.

This guards against #950: contracts declaring `pausable`/`upgradeable`/
`capped-supply`/`freeze`/`transfer-hook` as Cargo features that silently never
got enabled by the repo's own build tooling, shipping WASM with the safety and
governance entry points compiled out. See docs/deployment-guide.md, "Cargo
Feature Defaults", for what is expected to ship in a normal deployment.

Usage: python3 check_feature_exports.py [wasm_dir]
"""

import struct
import sys

# wasm file name -> exported function names its declared `default` Cargo
# features are expected to compile in. Keep in sync with each contract's
# Cargo.toml `[features] default = [...]` and docs/deployment-guide.md.
EXPECTED_EXPORTS = {
    "soroban_token_template.wasm": [
        "pause", "unpause",                      # pausable
        "propose_upgrade", "execute_upgrade",     # upgradeable
        "max_supply",                             # capped-supply
        "freeze_account", "unfreeze_account",     # freeze
        "set_transfer_hook",                      # transfer-hook
    ],
    "soroban_escrow_template.wasm": [
        "pause", "unpause",                       # pausable
        "propose_upgrade", "execute_upgrade",     # pausable (upgrade timelock)
    ],
    "wrapped_token.wasm": [
        "pause", "unpause",                       # pausable
    ],
    # soroban_multisig_template.wasm intentionally omitted: its `pausable` /
    # `upgradeable` Cargo features currently guard no production code (see
    # contracts/multisig/Cargo.toml), so there is nothing to assert on yet.
}


def read_uleb128(data, offset):
    result = 0
    shift = 0
    while True:
        byte = data[offset]
        offset += 1
        result |= (byte & 0x7F) << shift
        if not (byte & 0x80):
            return result, offset
        shift += 7


def exported_function_names(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[0:4] != b"\0asm":
        raise ValueError(f"{path} is not a WASM binary")
    offset = 8
    names = []
    while offset < len(data):
        section_id = data[offset]
        offset += 1
        size, offset = read_uleb128(data, offset)
        section_end = offset + size
        if section_id == 7:  # export section
            count, off = read_uleb128(data, offset)
            for _ in range(count):
                name_len, off = read_uleb128(data, off)
                name = data[off:off + name_len].decode("utf-8")
                off += name_len
                kind = data[off]
                off += 1
                _index, off = read_uleb128(data, off)
                if kind == 0:  # function export
                    names.append(name)
        offset = section_end
    return names


def main():
    wasm_dir = sys.argv[1] if len(sys.argv) > 1 else "target/wasm32-unknown-unknown/release"
    failed = False
    for wasm_file, expected in EXPECTED_EXPORTS.items():
        path = f"{wasm_dir}/{wasm_file}"
        try:
            exports = set(exported_function_names(path))
        except FileNotFoundError:
            print(f"ERROR: {path} not found — was it built?")
            failed = True
            continue
        missing = [fn for fn in expected if fn not in exports]
        if missing:
            print(f"ERROR: {wasm_file} is missing expected exports: {', '.join(missing)}")
            print("       (a default Cargo feature may have regressed — see .github/scripts/check_feature_exports.py)")
            failed = True
        else:
            print(f"OK: {wasm_file} exports all {len(expected)} expected feature-gated functions")
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
