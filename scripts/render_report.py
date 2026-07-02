#!/usr/bin/env python3
"""Render a self-contained HTML perf report (inline-SVG charts) from results.jsonl.

Stdlib only — runs anywhere python3 exists (macOS, CI runners) with zero installs.

    render_report.py --results <path/to/results.jsonl> \
                     [--history reports] [--out <path/REPORT.html>]

Charts:
  1. Read throughput (rows/s) per workload/codec        — horizontal bars
  2. Write + mixed throughput (ops/s) per workload/conc — horizontal bars
  3. Latency p50 vs p99 per workload                    — grouped bars, log scale
  4. Cross-version trend (full_scan rows/s, point_lookup p99) across every
     results.jsonl found under --history                — line charts
  5. If an intervals.jsonl sits next to the results (soak series, cqlite-perf #31),
     per-cohort time-series charts of throughput / p99 / RSS.

SUMMARY.md and SCORECARD.md (if present next to the results) are embedded verbatim.
"""

import argparse
import glob
import html
import json
import math
import os
import sys

# ── palette / layout ─────────────────────────────────────────────────────────
BAR = "#4c78a8"
BAR2 = "#f58518"
LINE = "#4c78a8"
GRID = "#e0e0e0"
TEXT = "#1a1a1a"
MUTED = "#666666"
W = 920  # chart width


def fmt(v):
    """Human number: 1234567 -> 1.23M, 43786 -> 43.8k."""
    if v is None:
        return "—"
    a = abs(v)
    for div, suf in ((1e9, "G"), (1e6, "M"), (1e3, "k")):
        if a >= div:
            return f"{v / div:.3g}{suf}"
    return f"{v:.3g}"


def esc(s):
    return html.escape(str(s), quote=True)


# ── SVG primitives ───────────────────────────────────────────────────────────

def svg_open(height, title):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {height}" '
        f'width="{W}" height="{height}" role="img" style="max-width:100%;height:auto">'
        f'<text x="8" y="20" font-size="15" font-weight="bold" fill="{TEXT}">{esc(title)}</text>'
    )


def hbar_chart(title, items, unit, log_scale=False):
    """items: [(label, value)] -> horizontal bar chart SVG. Sorted desc."""
    items = [(l, v) for l, v in items if v is not None and v > 0]
    if not items:
        return ""
    items.sort(key=lambda t: -t[1])
    row_h, top, label_w, right = 26, 36, 300, 90
    height = top + row_h * len(items) + 14
    vmax = max(v for _, v in items)
    lo = min(v for _, v in items)
    span_w = W - label_w - right

    def bar_w(v):
        if log_scale:
            lmin = math.log10(max(lo / 2, 1e-9))
            return span_w * (math.log10(v) - lmin) / (math.log10(vmax) - lmin or 1)
        return span_w * v / vmax

    out = [svg_open(height, f"{title} ({unit}{', log scale' if log_scale else ''})")]
    for i, (label, v) in enumerate(items):
        y = top + i * row_h
        bw = max(bar_w(v), 2)
        out.append(
            f'<text x="{label_w - 8}" y="{y + 16}" font-size="12" text-anchor="end" fill="{TEXT}">{esc(label)}</text>'
            f'<rect x="{label_w}" y="{y + 3}" width="{bw:.1f}" height="{row_h - 8}" fill="{BAR}" rx="2"/>'
            f'<text x="{label_w + bw + 6:.1f}" y="{y + 16}" font-size="12" fill="{MUTED}">{fmt(v)}</text>'
        )
    out.append("</svg>")
    return "".join(out)


def grouped_bar_chart(title, items, unit):
    """items: [(label, v1, v2)] two log-scale bars per row (p50 vs p99)."""
    items = [(l, a, b) for l, a, b in items if a and b]
    if not items:
        return ""
    items.sort(key=lambda t: -t[2])
    row_h, top, label_w, right = 40, 52, 300, 90
    height = top + row_h * len(items) + 14
    vals = [v for _, a, b in items for v in (a, b)]
    vmax, vmin = max(vals), min(vals)
    span_w = W - label_w - right
    lmin = math.log10(max(vmin / 2, 1e-9))
    lspan = math.log10(vmax) - lmin or 1

    def bw(v):
        return max(span_w * (math.log10(v) - lmin) / lspan, 2)

    out = [svg_open(height, f"{title} ({unit}, log scale)")]
    out.append(
        f'<rect x="{label_w}" y="30" width="12" height="10" fill="{BAR}"/>'
        f'<text x="{label_w + 17}" y="39" font-size="12" fill="{MUTED}">p50</text>'
        f'<rect x="{label_w + 60}" y="30" width="12" height="10" fill="{BAR2}"/>'
        f'<text x="{label_w + 77}" y="39" font-size="12" fill="{MUTED}">p99</text>'
    )
    for i, (label, p50, p99) in enumerate(items):
        y = top + i * row_h
        out.append(
            f'<text x="{label_w - 8}" y="{y + 22}" font-size="12" text-anchor="end" fill="{TEXT}">{esc(label)}</text>'
            f'<rect x="{label_w}" y="{y + 4}" width="{bw(p50):.1f}" height="12" fill="{BAR}" rx="2"/>'
            f'<text x="{label_w + bw(p50) + 6:.1f}" y="{y + 14}" font-size="11" fill="{MUTED}">{fmt(p50)}</text>'
            f'<rect x="{label_w}" y="{y + 19}" width="{bw(p99):.1f}" height="12" fill="{BAR2}" rx="2"/>'
            f'<text x="{label_w + bw(p99) + 6:.1f}" y="{y + 29}" font-size="11" fill="{MUTED}">{fmt(p99)}</text>'
        )
    out.append("</svg>")
    return "".join(out)


def line_chart(title, points, xlabels, unit, log_scale=False):
    """points: [(x_index, value)] over positions 0..len(xlabels)-1; gaps allowed."""
    points = [(x, v) for x, v in points if v is not None and v > 0]
    if len(points) < 2:
        return ""
    top, bottom, left, right = 36, 64, 90, 30
    height = 300
    plot_w, plot_h = W - left - right, height - top - bottom
    vals = [v for _, v in points]
    vmax, vmin = max(vals), min(vals)
    if log_scale:
        lmin, lspan = math.log10(vmin / 1.5), math.log10(vmax * 1.5) - math.log10(vmin / 1.5)
        vy = lambda v: top + plot_h * (1 - (math.log10(v) - lmin) / lspan)
    else:
        lo, span = 0.0, vmax * 1.1 or 1
        vy = lambda v: top + plot_h * (1 - (v - lo) / span)
    n = max(len(xlabels) - 1, 1)
    vx = lambda x: left + plot_w * x / n

    out = [svg_open(height, f"{title} ({unit}{', log scale' if log_scale else ''})")]
    # gridlines + y ticks (4)
    for i in range(5):
        gy = top + plot_h * i / 4
        if log_scale:
            gv = 10 ** (lmin + lspan * (1 - i / 4))
        else:
            gv = (vmax * 1.1) * (1 - i / 4)
        out.append(
            f'<line x1="{left}" y1="{gy:.1f}" x2="{W - right}" y2="{gy:.1f}" stroke="{GRID}"/>'
            f'<text x="{left - 8}" y="{gy + 4:.1f}" font-size="11" text-anchor="end" fill="{MUTED}">{fmt(gv)}</text>'
        )
    # x labels
    for i, lbl in enumerate(xlabels):
        out.append(
            f'<text x="{vx(i):.1f}" y="{height - 8}" font-size="11" text-anchor="end" fill="{MUTED}" '
            f'transform="rotate(-25 {vx(i):.1f} {height - 8})">{esc(lbl)}</text>'
        )
    path = " ".join(f"{'M' if i == 0 else 'L'}{vx(x):.1f},{vy(v):.1f}" for i, (x, v) in enumerate(points))
    out.append(f'<path d="{path}" fill="none" stroke="{LINE}" stroke-width="2.5"/>')
    for x, v in points:
        out.append(
            f'<circle cx="{vx(x):.1f}" cy="{vy(v):.1f}" r="4" fill="{LINE}"/>'
            f'<text x="{vx(x):.1f}" y="{vy(v) - 10:.1f}" font-size="11" text-anchor="middle" fill="{TEXT}">{fmt(v)}</text>'
        )
    out.append("</svg>")
    return "".join(out)


# ── data loading ─────────────────────────────────────────────────────────────

def load_jsonl(path):
    recs = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                try:
                    recs.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
    return recs


def rec_label(r, dims):
    """Label a record with only the dimensions that vary in this chart."""
    parts = [r["workload"]]
    if "codec" in dims:
        parts.append(r.get("dataset", {}).get("codec", "?"))
    if "concurrency" in dims:
        parts.append(f"conc={r.get('concurrency')}")
    return "  ".join(parts)


def varying_dims(recs):
    dims = set()
    if len({r.get("dataset", {}).get("codec") for r in recs}) > 1:
        dims.add("codec")
    if len({r.get("concurrency") for r in recs}) > 1:
        dims.add("concurrency")
    return dims


# ── report sections ──────────────────────────────────────────────────────────

def current_run_charts(recs):
    reads = [r for r in recs if r["workload"].startswith("read.")]
    others = [r for r in recs if not r["workload"].startswith("read.")]
    charts = []
    if reads:
        dims = varying_dims(reads)
        charts.append(hbar_chart(
            "Read throughput", [(rec_label(r, dims), r["throughput"].get("rows_per_sec")) for r in reads],
            "rows/s", log_scale=True))
    if others:
        dims = varying_dims(others)
        charts.append(hbar_chart(
            "Write + mixed throughput", [(rec_label(r, dims), r["throughput"].get("ops_per_sec")) for r in others],
            "ops/s", log_scale=True))
    dims = varying_dims(recs)
    charts.append(grouped_bar_chart(
        "Per-op latency", [(rec_label(r, dims), r["latency_us"].get("p50"), r["latency_us"].get("p99")) for r in recs],
        "µs"))
    return [c for c in charts if c]


def trend_charts(history_root, current_path):
    """One point per results.jsonl under history_root (plus current), sorted by dir name."""
    files = sorted(glob.glob(os.path.join(history_root, "*", "results.jsonl")))
    cur = os.path.abspath(current_path)
    if cur not in (os.path.abspath(f) for f in files):
        files.append(current_path)
    runs = []
    for f in files:
        recs = load_jsonl(f)
        if not recs:
            continue
        date = os.path.basename(os.path.dirname(f))[:10]
        ver = recs[0].get("cqlite_version", "?")
        runs.append((date, ver, recs))
    if len(runs) < 2:
        return []

    def pick(recs, workload, metric_path):
        cands = [r for r in recs if r["workload"] == workload
                 and r.get("dataset", {}).get("codec") in (None, "lz4")]
        if not cands:
            return None
        r = max(cands, key=lambda r: r["throughput"].get("rows_per_sec") or 0)
        v = r
        for k in metric_path:
            v = (v or {}).get(k)
        return v

    xlabels = [f"{d} {v}" for d, v, _ in runs]
    specs = [
        ("read.full_scan throughput across engine versions", "read.full_scan",
         ("throughput", "rows_per_sec"), "rows/s", False),
        ("read.point_lookup p99 across engine versions", "read.point_lookup",
         ("latency_us", "p99"), "µs", True),
    ]
    charts = []
    for title, wl, path, unit, log in specs:
        pts = [(i, pick(recs, wl, path)) for i, (_, _, recs) in enumerate(runs)]
        charts.append(line_chart(title, pts, xlabels, unit, log_scale=log))
    return [c for c in charts if c]


def interval_charts(intervals_path):
    """Soak time-series (cqlite-perf #31): one chart per (workload, cohort, metric)."""
    recs = load_jsonl(intervals_path)
    if not recs:
        return []
    charts = []
    groups = {}
    for r in recs:
        groups.setdefault((r.get("workload", "?"), r.get("cohort", "all")), []).append(r)
    for (wl, cohort), rows in sorted(groups.items()):
        rows.sort(key=lambda r: r.get("elapsed_s", 0))
        xlabels = [f"{r.get('elapsed_s', 0) / 60:.0f}m" for r in rows]
        for title, get, unit, log in (
            ("throughput", lambda r: r.get("throughput_ops_per_sec"), "ops/s", False),
            ("p99", lambda r: (r.get("latency_us") or {}).get("p99"), "µs", True),
            ("RSS", lambda r: (r.get("rss_bytes") or 0) / 1e6 or None, "MB", False),
        ):
            pts = [(i, get(r)) for i, r in enumerate(rows)]
            charts.append(line_chart(f"soak · {wl} [{cohort}] — {title} over time", pts, xlabels, unit, log_scale=log))
    return [c for c in charts if c]


def embed_md(path, title):
    if not os.path.exists(path):
        return ""
    with open(path) as f:
        body = f.read()
    return (f"<details open><summary><b>{esc(title)}</b></summary>"
            f'<pre class="md">{esc(body)}</pre></details>')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--results", required=True, help="results.jsonl of the current run")
    ap.add_argument("--history", default="reports", help="root dir scanned for */results.jsonl trend points")
    ap.add_argument("--out", default=None, help="output HTML (default: REPORT.html next to results)")
    args = ap.parse_args()

    recs = load_jsonl(args.results)
    if not recs:
        sys.exit(f"no records in {args.results}")
    run_dir = os.path.dirname(os.path.abspath(args.results))
    out_path = args.out or os.path.join(run_dir, "REPORT.html")

    r0 = recs[0]
    host = r0.get("host", {})
    meta = (f'cqlite <b>{esc(r0.get("cqlite_version"))}</b> · harness {esc(r0.get("harness_version"))} · '
            f'{esc(host.get("cpu"))}/{host.get("cores")}c {esc(host.get("os"))} · '
            f'{len(recs)} results · seed {r0.get("seed")}')

    sections = []
    sections.append("<h2>This run</h2>")
    sections += current_run_charts(recs)

    trends = trend_charts(args.history, args.results)
    if trends:
        sections.append("<h2>Cross-version trend</h2>"
                        "<p class='note'>One point per report under <code>reports/</code> "
                        "(basic/S/lz4, conc-best). Host signatures may differ across points — "
                        "treat shape, not absolutes, as the signal (TEST_PLAN Reference C).</p>")
        sections += trends

    ivals = os.path.join(run_dir, "intervals.jsonl")
    if os.path.exists(ivals):
        sections.append("<h2>Soak time series</h2>")
        sections += interval_charts(ivals)

    sections.append("<h2>Artifacts</h2>")
    sections.append(embed_md(os.path.join(run_dir, "SCORECARD.md"), "SCORECARD.md"))
    sections.append(embed_md(os.path.join(run_dir, "SUMMARY.md"), "SUMMARY.md"))

    doc = f"""<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>cqlite-perf report — {esc(r0.get("cqlite_version"))} — {esc(os.path.basename(run_dir))}</title>
<style>
 body {{ font: 14px/1.5 -apple-system, "Segoe UI", Helvetica, Arial, sans-serif;
        color: {TEXT}; max-width: 980px; margin: 24px auto; padding: 0 16px; }}
 h1 {{ font-size: 22px; margin-bottom: 2px; }}
 h2 {{ font-size: 17px; border-bottom: 1px solid {GRID}; padding-bottom: 4px; margin-top: 32px; }}
 .meta {{ color: {MUTED}; margin-bottom: 20px; }}
 .note {{ color: {MUTED}; font-size: 12.5px; }}
 svg {{ display: block; margin: 18px 0; }}
 pre.md {{ background: #f6f8fa; padding: 12px; overflow-x: auto; font-size: 12px; border-radius: 6px; }}
 details {{ margin: 12px 0; }}
</style></head><body>
<h1>cqlite-perf report</h1>
<div class="meta">{meta}</div>
{"".join(sections)}
</body></html>"""

    with open(out_path, "w") as f:
        f.write(doc)
    print(f"✓ report: {out_path}")


if __name__ == "__main__":
    main()
