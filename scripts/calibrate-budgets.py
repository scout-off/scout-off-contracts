#!/usr/bin/env python3
"""
Budget calibration helper for issue #823.

Reads the current measured WASM sizes and CPU-cost report produced by CI and
prints a recommended tightened budget (measured + headroom) for each contract
and operation.

Usage:
    python scripts/calibrate-budgets.py [--headroom 0.20]

The default headroom is 20%.  Pass a smaller value (e.g. 0.10) for tighter
budgets, or 0.0 to recommend the raw measured values.
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WASM_SIZES_PATH = REPO_ROOT / "abi" / "wasm-sizes.json"
CPU_REPORT_PATH = REPO_ROOT / "cpu-cost-budget-report.txt"
WASM_BUDGET_PATH = REPO_ROOT / "ci" / "wasm-size-budget.json"
CPU_BUDGET_PATH = REPO_ROOT / "ci" / "cpu-cost-budget.md"


def load_json(path: Path):
    if not path.exists():
        print(f"ERROR: {path} not found. Run the CI jobs first.", file=sys.stderr)
        sys.exit(1)
    with open(path) as f:
        return json.load(f)


def parse_cpu_report(path: Path) -> dict[str, dict[str, int]]:
    """Parse the --nocapture cost_budget.rs report into a nested map.

    Supports two output formats:
    - New format (println! from assert_cpu_budget):
        cost_budget: progress::advance_level = 403752 cpu instructions (budget 15000000)
    - Legacy format (cargo test output):
        scoutchain_scout_access::tests::cost_budget::test_subscribe_cost ... passed: 3_421_000
    """
    if not path.exists():
        return {}
    text = path.read_text()
    results: dict[str, dict[str, int]] = {}
    for line in text.splitlines():
        # New format: cost_budget: <contract>::<op> = <cpu> cpu instructions (budget <budget>)
        m = re.search(r"cost_budget:\s+(\w+)::(\w+)\s*=\s*(\d+)\s+cpu instructions", line)
        if m:
            contract, op, cost = m.group(1), m.group(2), int(m.group(3))
            results.setdefault(contract, {})[op] = cost
            continue
        # Legacy format: scoutchain_<contract>::tests::cost_budget::test_<op>_cost ... passed: <cost>
        m = re.search(r"scoutchain_(\w+)::tests::cost_budget::test_(\w+)_cost", line)
        if m:
            contract, op = m.group(1), m.group(2)
            cost_match = re.search(r"passed:\s*([\d_]+)", line)
            if cost_match:
                cost = int(cost_match.group(1).replace("_", ""))
                results.setdefault(contract, {})[op] = cost
    return results


def recommend_wasm_budgets(measured: dict[str, int], headroom: float) -> dict[str, int]:
    return {contract: int(size * (1 + headroom)) for contract, size in measured.items()}


def recommend_cpu_budgets(measured: dict[str, dict[str, int]], headroom: float) -> dict[str, dict[str, int]]:
    return {
        contract: {op: int(cost * (1 + headroom)) for op, cost in ops.items()}
        for contract, ops in measured.items()
    }


def update_wasm_budget_file(recommended: dict[str, int]) -> None:
    data = load_json(WASM_BUDGET_PATH)
    data["budgets"] = recommended
    with open(WASM_BUDGET_PATH, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print(f"Updated {WASM_BUDGET_PATH}")


def update_cpu_budget_md(recommended: dict[str, dict[str, int]]) -> None:
    if not CPU_BUDGET_PATH.exists():
        return
    lines = CPU_BUDGET_PATH.read_text().splitlines()
    new_lines = []
    in_table = False
    for line in lines:
        if line.startswith("| Contract"):
            in_table = True
            new_lines.append(line)
            continue
        if in_table and line.startswith("|--"):
            in_table = False
            new_lines.append(line)
            continue
        if in_table and line.startswith("|"):
            parts = [p.strip() for p in line.strip("|").split("|")]
            if len(parts) >= 3:
                contract = parts[0].strip()
                op = parts[1].strip().strip("`")
                if contract in recommended and op in recommended[contract]:
                    new_budget = recommended[contract][op]
                    new_line = f"| {contract:<13} | {op:<35} | {new_budget:>25} |"
                    new_lines.append(new_line)
                    continue
        new_lines.append(line)
    CPU_BUDGET_PATH.write_text("\n".join(new_lines) + "\n")
    print(f"Updated {CPU_BUDGET_PATH}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Calibrate WASM and CPU cost budgets from CI measurements")
    parser.add_argument("--headroom", type=float, default=0.20, help="Fractional headroom to add (default: 0.20)")
    args = parser.parse_args()

    if WASM_SIZES_PATH.exists():
        wasm_sizes = load_json(WASM_SIZES_PATH)
    else:
        wasm_sizes = {}
        print(f"WARNING: {WASM_SIZES_PATH} not found, skipping WASM budgets.", file=sys.stderr)
    cpu_report = parse_cpu_report(CPU_REPORT_PATH)

    wasm_rec = recommend_wasm_budgets(wasm_sizes, args.headroom)
    cpu_rec = recommend_cpu_budgets(cpu_report, args.headroom)

    print("=== Recommended WASM size budgets (bytes) ===")
    for contract, size in sorted(wasm_rec.items()):
        measured = wasm_sizes.get(contract, 0)
        print(f"  {contract:<14} measured={measured:>8}  recommended={size:>8}  headroom={args.headroom:.0%}")

    print("\n=== Recommended CPU cost budgets (instructions) ===")
    for contract, ops in sorted(cpu_rec.items()):
        for op, budget in sorted(ops.items()):
            measured = cpu_report.get(contract, {}).get(op, 0)
            print(f"  {contract:<14} {op:<35} measured={measured:>12}  recommended={budget:>12}")

    # Optionally write back
    if os.environ.get("CALIBRATE_WRITE") == "1":
        if wasm_sizes:
            update_wasm_budget_file(wasm_rec)
        update_cpu_budget_md(cpu_rec)
    else:
        print("\nSet CALIBRATE_WRITE=1 to update ci/wasm-size-budget.json and ci/cpu-cost-budget.md in-place.")


if __name__ == "__main__":
    main()
