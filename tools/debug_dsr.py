#!/usr/bin/env python3
"""Debug DSR computation - check expected max Sharpe and PSR. No mpmath needed."""
import csv, math

path = '/root/depthscope_out/sweepscope_scorecard.csv'
with open(path) as f:
    reader = csv.DictReader(f)
    rows = [r for r in reader if int(r['trades']) >= 30]

# Collect all Sharpes
sharpes = [float(r['sharpe']) for r in rows]
n = len(sharpes)
mean_sr = sum(sharpes) / n
var_sr = sum((s - mean_sr)**2 for s in sharpes) / n

print(f"N configs: {n}")
print(f"Mean Sharpe: {mean_sr:.4f}")
print(f"Var Sharpes: {var_sr:.6f}")
print(f"Max Sharpe: {max(sharpes):.4f}")
print(f"Min Sharpe: {min(sharpes):.4f}")

# Approximate inv_normal_cdf using scipy-free rational approximation (Abramowitz & Stegun)
def inv_normal_cdf(p):
    """Rational approximation for the inverse normal CDF (Abramowitz & Stegun 26.2.23)."""
    if p <= 0: return float('-inf')
    if p >= 1: return float('inf')
    if p == 0.5: return 0.0
    if p > 0.5:
        return -inv_normal_cdf(1 - p)
    t = math.sqrt(-2 * math.log(p))
    c0, c1, c2 = 2.515517, 0.802853, 0.010328
    d1, d2, d3 = 1.432788, 0.189269, 0.001308
    return -(t - (c0 + c1*t + c2*t*t) / (1 + d1*t + d2*t*t + d3*t*t*t))

# Expected max Sharpe (Bailey-LdP)
euler = 0.5772156649015329
n_f = max(n, 2)
e_max = math.sqrt(max(var_sr, 0)) * (
    (1 - euler) * inv_normal_cdf(1 - 1/n_f) +
    euler * inv_normal_cdf(1 - 1/(n_f * math.e))
)
print(f"\nExpected max Sharpe (null): {e_max:.4f}")
print(f"Top config Sharpe: {max(sharpes):.4f}")
print(f"Gap (Sharpe - E[max]): {max(sharpes) - e_max:.4f}")
print(f"\nIf E[max] > observed Sharpe, DSR will be ~0")

# Also check: what if we only had 100 trials?
for n_trials in [1, 5, 10, 50, 100, 500, 3257]:
    e = math.sqrt(max(var_sr, 0)) * (
        (1 - euler) * inv_normal_cdf(1 - 1/max(n_trials, 2)) +
        euler * inv_normal_cdf(1 - 1/(max(n_trials, 2) * math.e))
    )
    print(f"  n_trials={n_trials:>5}: E[max] = {e:.4f}")
