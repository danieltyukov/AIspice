#!/usr/bin/env bash
# Regenerate the simulator output fixtures from the decks in decks/.
#
# Needs ngspice, Xyce and LTspice XVII under Wine (with xvfb-run). The outputs
# are results of our own decks, small enough to commit, and the tests in
# tests/raw_fixtures.rs and tests/log_fixtures.rs check them against analytic
# values, so a regenerated set must still pass those tests.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
decks="$here/decks"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

ltspice="${LTSPICE_EXE:-$HOME/.wine/drive_c/Program Files/LTC/LTspiceXVII/XVIIx64.exe}"

# ngspice: binary raw, ASCII raw for the multi-plot deck, stdout for logs.
mkdir -p "$here/ngspice" "$work/ng"
for d in rc_tran rc_ac div_op div_dc multi; do
    cp "$decks/$d.cir" "$work/ng/"
    (cd "$work/ng" && ngspice -b -r "$d.raw" "$d.cir" >/dev/null 2>&1)
    cp "$work/ng/$d.raw" "$here/ngspice/"
done
(cd "$work/ng" && SPICE_ASCIIRAWFILE=1 ngspice -b -r multi_ascii.raw multi.cir >/dev/null 2>&1)
cp "$work/ng/multi_ascii.raw" "$here/ngspice/"
for d in ng_meas bad_subckt; do
    cp "$decks/$d.cir" "$work/ng/"
    (cd "$work/ng" && ngspice -b "$d.cir" >"$d.stdout" 2>&1 || true)
    cp "$work/ng/$d.stdout" "$here/ngspice/"
done

# Xyce: binary and ASCII raw, stdout for logs.
mkdir -p "$here/xyce" "$work/xy"
for d in rc_tran rc_ac div_op div_dc xyce_step; do
    cp "$decks/$d.cir" "$work/xy/"
    (cd "$work/xy" && Xyce -r "$d.raw" "$d.cir" >/dev/null 2>&1)
    (cd "$work/xy" && Xyce -r "${d}_ascii.raw" -a "$d.cir" >/dev/null 2>&1)
    cp "$work/xy/$d.raw" "$work/xy/${d}_ascii.raw" "$here/xyce/"
done
cp "$decks/bad_subckt.cir" "$decks/xyce_meas.cir" "$work/xy/"
(cd "$work/xy" && Xyce bad_subckt.cir >bad_subckt.stdout 2>&1 || true)
(cd "$work/xy" && Xyce xyce_meas.cir >xyce_meas.stdout 2>&1)
cp "$work/xy/bad_subckt.stdout" "$work/xy/xyce_meas.stdout" "$work/xy/xyce_meas.cir.mt0" "$here/xyce/"

# LTspice under Wine, all decks in parallel under one virtual display.
mkdir -p "$here/ltspice" "$work/lt"
lt_decks=(rc_tran rc_ac div_op div_dc rc_step div_step_op rc_ac_step rc_meas
    rc_tran_double rc_tran_offset rc_noise div_tf op_tran bad_subckt floating
    meas_forms meas_ac meas_step_fail)
for d in "${lt_decks[@]}"; do cp "$decks/$d.cir" "$work/lt/"; done
mkdir -p "$work/lt/ascii" "$work/lt/fast"
cp "$decks/rc_ac.cir" "$decks/rc_ascii.cir" "$work/lt/ascii/"
cp "$decks/rc_step.cir" "$work/lt/fast/"
cat >"$work/lt/run.sh" <<EOF
cd "$work/lt"
for d in ${lt_decks[*]}; do wine "$ltspice" -b "\$d.cir" >/dev/null 2>&1 & done
(cd ascii && wine "$ltspice" -b -ascii rc_ac.cir >/dev/null 2>&1; wine "$ltspice" -b -ascii rc_ascii.cir >/dev/null 2>&1) &
(cd fast && wine "$ltspice" -b rc_step.cir >/dev/null 2>&1 && wine "$ltspice" -FastAccess rc_step.raw >/dev/null 2>&1) &
wait
EOF
xvfb-run -a bash "$work/lt/run.sh"
for f in "$work"/lt/*.raw "$work"/lt/*.log; do cp "$f" "$here/ltspice/"; done
cp "$work/lt/ascii/rc_ac.raw" "$here/ltspice/rc_ac_ascii.raw"
cp "$work/lt/ascii/rc_ascii.raw" "$work/lt/ascii/rc_ascii.log" "$here/ltspice/"
cp "$work/lt/fast/rc_step.raw" "$here/ltspice/rc_step_fast.raw"

du -sh "$here/ngspice" "$here/xyce" "$here/ltspice"
