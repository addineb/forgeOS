#!/usr/bin/env python3
"""Check ETH data directory structure"""
import os, json

base = "/root/chd/data/ticks"
for root, dirs, files in os.walk(base):
    depth = root.replace(base, "").count("/")
    if depth <= 2:
        parquets = [f for f in files if f.endswith(".parquet")]
        print(f"{root} -> {len(dirs)} dirs, {len(parquets)} parquets")

print()
# Specifically check ETH and ETHUSDT
for path in ["ETH", "ETHUSDT"]:
    p = os.path.join(base, path)
    if os.path.exists(p):
        print(f"{p}: {os.listdir(p)}")
