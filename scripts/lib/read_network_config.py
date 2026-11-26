#!/usr/bin/env python3
"""Read and validate config/<network>.json for deploy.sh and initialize.sh.

Usage: read_network_config.py <config.json> [xlm_token_override]

Rejects missing/placeholder fields, enforces the bounds in
docs/CONFIG_REFERENCE.md, and prints: <xlm_token> <reg_cooldown_secs> <fee_config_json>.
A non-empty xlm_token_override (XLM_TOKEN_ADDRESS) replaces xlm_token_address.
"""
import json, sys
path = sys.argv[1]
token_env = sys.argv[2] if len(sys.argv) > 2 else ""
cfg = json.load(open(path))
def fail(msg):
    sys.exit(f"ERROR: {path}: {msg}")
def get(d, k):
    v = d.get(k)
    if v is None or (isinstance(v, str) and (not v or "FILL_IN_BEFORE_USE" in v)):
        fail(f"'{k}' is missing or still a placeholder")
    return v
token = token_env or get(cfg, "xlm_token_address")
cool = get(cfg, "reg_cooldown_secs")
if not isinstance(cool, int) or not 0 <= cool <= 604800:
    fail("reg_cooldown_secs must be an integer in 0..=604800")
fc = get(cfg, "fee_config")
for k in ("contact_fee_stroops", "basic_sub_stroops", "pro_sub_stroops", "elite_sub_stroops",
          "trial_offer_escrow_stroops"):
    try:
        fc[k] = int(get(fc, k))
    except (TypeError, ValueError):
        fail(f"fee_config.{k} must be an integer")
for k in ("sub_duration_secs", "trial_offer_expiry_secs", "pro_contact_limit"):
    if not isinstance(get(fc, k), int):
        fail(f"fee_config.{k} must be an integer")
def bound(k, lo, hi):
    if not lo <= fc[k] <= hi:
        fail(f"fee_config.{k}={fc[k]} outside {lo}..={hi}")
bound("contact_fee_stroops", 100_000, 100_000_000_000)
for k in ("basic_sub_stroops", "pro_sub_stroops", "elite_sub_stroops"):
    bound(k, 1_000_000, 1_000_000_000_000)
if not fc["basic_sub_stroops"] <= fc["pro_sub_stroops"] <= fc["elite_sub_stroops"]:
    fail("fee_config subscription prices must satisfy basic <= pro <= elite")
bound("sub_duration_secs", 1, 365 * 24 * 3600)
bound("pro_contact_limit", 1, 10_000)
if fc["trial_offer_escrow_stroops"] < 0 or (fc["trial_offer_escrow_stroops"] > 0 and fc["trial_offer_expiry_secs"] == 0):
    fail("fee_config trial offer escrow must be >= 0 and needs a non-zero expiry when > 0")
fc = {k: (str(v) if k.endswith("_stroops") else v) for k, v in fc.items()}
print(token, cool, json.dumps(fc, separators=(",", ":")))
