#!/usr/bin/env python3
"""Realistic P&L simulation for CVD reversion signal with €500 starting balance."""

import pandas as pd
import numpy as np
import sys

def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/root/depthscope_out/stitched_vb10.csv"
    df = pd.read_csv(path)
    df['utc_date'] = pd.to_datetime(df['ts'], unit='ns').dt.date

    print("=" * 70)
    print("CVD REVERSION: REALISTIC P&L SIMULATION (€500 START)")
    print("=" * 70)

    # Signal parameters from threshold optimization
    LONG_THRESHOLD = -3500  # go long when cvd_delta < this
    HOLD_HORIZON = 'fwd_ret_15m_bps'  # 15min hold

    # Filter signal bars
    signals = df[df['cvd_delta'] < LONG_THRESHOLD].copy()
    print(f"\nSignal: LONG when cvd_delta < {LONG_THRESHOLD}")
    print(f"Hold: 15 minutes (volume bar to volume bar)")
    print(f"Total signals: {len(signals)} over {df['utc_date'].nunique()} dates")
    print(f"Avg signals/day: {len(signals) / df['utc_date'].nunique():.0f}")

    # === PER-TRADE P&L DISTRIBUTION ===
    print("\n--- PER-TRADE P&L (taker, 4.5 bps one-way) ---")
    rets = signals[HOLD_HORIZON].values
    # Taker: pay 4.5 bps to enter, 4.5 bps to exit = 9 bps round-trip
    pnl_taker = rets - 9.0  # gross return minus round-trip fee
    # Maker: pay 2.5 bps to enter, 2.5 bps to exit = 5 bps round-trip
    pnl_maker = rets - 5.0

    wins_taker = pnl_taker > 0
    wins_maker = pnl_maker > 0

    print(f"  Taker (9 bps RT):")
    print(f"    Win rate: {wins_taker.mean():.1%}")
    print(f"    Avg win:  {pnl_taker[wins_taker].mean():.2f} bps")
    print(f"    Avg loss: {pnl_taker[~wins_taker].mean():.2f} bps")
    print(f"    Avg trade: {pnl_taker.mean():.2f} bps")
    print(f"    Median trade: {np.median(pnl_taker):.2f} bps")
    print(f"    Worst trade: {pnl_taker.min():.2f} bps")
    print(f"    Best trade: {pnl_taker.max():.2f} bps")
    print(f"    Risk-reward ratio: {abs(pnl_taker[wins_taker].mean() / pnl_taker[~wins_taker].mean()):.2f}:1")

    print(f"\n  Maker (5 bps RT):")
    print(f"    Win rate: {wins_maker.mean():.1%}")
    print(f"    Avg win:  {pnl_maker[wins_maker].mean():.2f} bps")
    print(f"    Avg loss: {pnl_maker[~wins_maker].mean():.2f} bps")
    print(f"    Avg trade: {pnl_maker.mean():.2f} bps")
    print(f"    Risk-reward ratio: {abs(pnl_maker[wins_maker].mean() / pnl_maker[~wins_maker].mean()):.2f}:1")

    # === EXPECTANCY ===
    print("\n--- EXPECTANCY ---")
    for label, pnl in [("Taker", pnl_taker), ("Maker", pnl_maker)]:
        ev = pnl.mean()
        ev_per_euro = ev / 10000  # bps to fraction
        print(f"  {label}: EV = {ev:.2f} bps/trade = €{ev_per_euro:.4f} per €1 risked")

    # === EQUITY CURVE SIMULATION ===
    print("\n--- EQUITY CURVE (€500 start, 100% position = €500 per trade) ---")
    for label, pnl in [("Taker (9bps RT)", pnl_taker), ("Maker (5bps RT)", pnl_maker)]:
        # Simulate with different position sizes
        for pos_pct in [20, 50, 100]:
            equity = 500.0
            max_equity = 500.0
            max_dd = 0.0
            position_eur = 500.0 * pos_pct / 100  # fixed position size in EUR
            
            equity_curve = [equity]
            for p in pnl:
                trade_pnl_eur = position_eur * (p / 10000)
                equity += trade_pnl_eur
                equity_curve.append(equity)
                if equity > max_equity:
                    max_equity = equity
                dd = (max_equity - equity) / max_equity * 100
                if dd > max_dd:
                    max_dd = dd
            
            final = equity
            total_return = (final - 500) / 500 * 100
            # Sharpe-like: mean pnl / std pnl * sqrt(trades per year ~ 460*12)
            sharpe_annual = pnl.mean() / pnl.std() * np.sqrt(460 * 12) if pnl.std() > 0 else 0
            
            print(f"  {label}, {pos_pct}% position (€{position_eur:.0f}/trade):")
            print(f"    Final equity: €{final:.2f} ({total_return:+.1f}%)")
            print(f"    Max drawdown: {max_dd:.1f}%")
            print(f"    Annualized Sharpe: {sharpe_annual:.2f}")
            print(f"    Total trades: {len(pnl)}")

    # === KELLY CRITERION ===
    print("\n--- KELLY CRITERION (optimal position sizing) ---")
    for label, pnl in [("Taker", pnl_taker), ("Maker", pnl_maker)]:
        w = (pnl > 0).mean()
        avg_w = pnl[pnl > 0].mean() / 10000  # as fraction
        avg_l = abs(pnl[pnl <= 0].mean()) / 10000  # as fraction
        if avg_l > 0:
            kelly = w / avg_l - (1 - w) / avg_w if avg_w > 0 else 0
            kelly = max(0, min(kelly, 1))  # clamp
        else:
            kelly = 0
        half_kelly = kelly / 2
        print(f"  {label}: Win%={w:.1%}, AvgWin={avg_w*10000:.1f}bps, AvgLoss={avg_l*10000:.1f}bps")
        print(f"    Full Kelly: {kelly:.1%} of bankroll")
        print(f"    Half Kelly: {half_kelly:.1%} of bankroll (recommended)")
        print(f"    Half Kelly position from €500: €{500 * half_kelly:.0f}")

    # === DAILY BREAKDOWN ===
    print("\n--- DAILY P&L BREAKDOWN (taker, long-only, cvd < -3500) ---")
    print(f"{'Date':>12} {'Trades':>7} {'Wins':>6} {'Win%':>6} {'Net_bps':>9} {'Net_EUR':>9} {'Cum_EUR':>10}")
    cum_eur = 500.0
    position_eur = 250.0  # 50% of starting balance
    
    for date in sorted(df['utc_date'].unique()):
        day_signals = df[(df['utc_date'] == date) & (df['cvd_delta'] < LONG_THRESHOLD)]
        if len(day_signals) == 0:
            continue
        day_pnl = day_signals[HOLD_HORIZON].values - 9.0  # taker
        day_wins = (day_pnl > 0).sum()
        day_net_bps = day_pnl.mean() if len(day_pnl) > 0 else 0
        day_net_eur = position_eur * day_pnl.sum() / 10000
        cum_eur += day_net_eur
        print(f"  {str(date):>12} {len(day_signals):>7} {day_wins:>6} {day_wins/len(day_signals):>6.1%} "
              f"{day_net_bps:>9.2f} {day_net_eur:>9.2f} {cum_eur:>10.2f}")

    # === WORST DRAWDOWN SCENARIOS ===
    print("\n--- WORST CASE SCENARIOS ---")
    # Consecutive losses
    consec_losses = 0
    max_consec = 0
    for p in pnl_taker:
        if p <= 0:
            consec_losses += 1
            max_consec = max(max_consec, consec_losses)
        else:
            consec_losses = 0
    print(f"  Max consecutive losses (taker): {max_consec}")
    
    # Worst 10 trades
    worst10 = np.sort(pnl_taker)[:10]
    print(f"  Worst 10 trades (bps): {', '.join(f'{x:.1f}' for x in worst10)}")
    print(f"  Worst 10 trades total: {worst10.sum():.1f} bps")
    print(f"  On €250 position: €{250 * worst10.sum() / 10000:.2f} loss")
    print(f"  On €500 position: €{500 * worst10.sum() / 10000:.2f} loss")

    # === REALITY CHECK ===
    print("\n--- REALITY CHECK ---")
    print("  ⚠ These are VOLUME-BAR returns, not trade P&L.")
    print("  ⚠ Volume bars are every 10 BTC traded, not every trade you make.")
    print("  ⚠ You can't enter at the exact bar start price.")
    print("  ⚠ Slippage on HL taker: ~0.5-2 bps depending on size.")
    print("  ⚠ This is IN-SAMPLE. OOS validation needed.")
    print("  ⚠ 3/18 dates had negative net (April 2-3, June 6).")
    print(f"  ⚠ Signal fires {len(signals)} times in 18 days = {len(signals)/18:.0f}/day avg.")
    print(f"  ⚠ At 50% position (€250), avg daily P&L = €{250 * pnl_taker.mean() * (len(signals)/18) / 10000:.2f}")

if __name__ == "__main__":
    main()