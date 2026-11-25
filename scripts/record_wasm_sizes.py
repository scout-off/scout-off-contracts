#!/usr/bin/env python3
import json
import sys
import os
import subprocess


def wasm_paths(contract, wasm_dir):
    """Return (source, optimized) WASM paths for a contract."""
    wasm_src = os.path.join(wasm_dir, f"scoutchain_{contract}.wasm")
    wasm_opt = os.path.join(wasm_dir, f"scoutchain_{contract}.optimized.wasm")
    return wasm_src, wasm_opt


def optimize_wasm(wasm_src, wasm_opt):
    """Optimize the WASM binary in place, raising on failure."""
    subprocess.run(
        [
            "stellar",
            "contract",
            "optimize",
            "--wasm",
            wasm_src,
            "--wasm-out",
            wasm_opt,
        ],
        check=True,
    )


def load_sizes(sizes_file):
    """Load existing size map, or an empty map when absent."""
    if os.path.exists(sizes_file):
        with open(sizes_file) as f:
            return json.load(f)
    return {}


def record_size(data, contract, size):
    """Record a contract's WASM size into the size map."""
    data[contract] = size
    return data


def write_sizes(sizes_file, data):
    """Write the size map back with stable formatting."""
    with open(sizes_file, "w") as f:
        json.dump(data, f, indent=2, sort_keys=True)


def main():
    # Get contract name from command line argument
    if len(sys.argv) < 2:
        print("Usage: record_wasm_sizes.py <contract_name>", file=sys.stderr)
        sys.exit(1)

    contract = sys.argv[1]
    wasm_dir = os.environ.get("WASM", "target/wasm32v1-none/release")
    wasm_src, wasm_opt = wasm_paths(contract, wasm_dir)
    sizes_file = "abi/wasm-sizes.json"

    # Optimize WASM
    optimize_wasm(wasm_src, wasm_opt)

    # Get size
    size = os.path.getsize(wasm_opt)

    # Read existing sizes or start fresh
    data = load_sizes(sizes_file)

    # Record size
    record_size(data, contract, size)

    # Write back
    write_sizes(sizes_file, data)


if __name__ == "__main__":
    main()