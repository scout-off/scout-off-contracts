#!/usr/bin/env python3
import json
import sys
import os

def main():
    # Get contract name from command line argument
    if len(sys.argv) < 2:
        print("Usage: record_wasm_sizes.py <contract_name>", file=sys.stderr)
        sys.exit(1)

    contract = sys.argv[1]
    wasm_dir = os.environ.get("WASM", "target/wasm32v1-none/release")
    wasm_src = os.path.join(wasm_dir, f"scoutchain_{contract}.wasm")
    wasm_opt = os.path.join(wasm_dir, f"scoutchain_{contract}.optimized.wasm")
    sizes_file = "abi/wasm-sizes.json"

    # Optimize WASM
    os.system(f"stellar contract optimize --wasm {wasm_src} --wasm-out {wasm_opt}")

    # Get size
    size = os.path.getsize(wasm_opt)

    # Read existing sizes or start fresh
    if os.path.exists(sizes_file):
        with open(sizes_file) as f:
            data = json.load(f)
    else:
        data = {}

    # Record size
    data[contract] = size

    # Write back
    with open(sizes_file, "w") as f:
        json.dump(data, f, indent=2, sort_keys=True)

if __name__ == "__main__":
    main()