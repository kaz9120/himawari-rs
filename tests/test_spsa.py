"""SPSAの数理を検証する（ADR-0143）。

スケジュール・摂動・更新の3つを、fishtest互換の性質で確かめる。
対局の起動は含まない。乱数は決定論なので、再開しても同じ摂動列に
なることをここで保証する。
"""

from hmwr import spsa_core
from hmwr.spsa_core import Param

P = Param(name="RFP_MULT", default=76.0, lo=20.0, hi=200.0, c_end=9.0, r_end=0.002)
Q = Param(name="LMR_COEF", default=2763.0, lo=1000.0, hi=6000.0, c_end=250.0, r_end=0.002)


def test_schedule_reaches_terminal_values():
    # 最終ペアでc・rとも終端値そのものになる
    c_mult, r_mult = spsa_core.schedule(15000, 15000)
    assert c_mult == 1.0
    assert r_mult == 1.0


def test_schedule_decays():
    c1, r1 = spsa_core.schedule(1, 15000)
    c2, r2 = spsa_core.schedule(7500, 15000)
    assert c1 > c2 > 1.0
    assert r1 > r2 > 1.0


def test_deltas_are_deterministic_and_pm1():
    d1 = spsa_core.deltas(42, 100, [P, Q])
    d2 = spsa_core.deltas(42, 100, [P, Q])
    assert d1 == d2
    assert set(d1) == {"RFP_MULT", "LMR_COEF"}
    assert all(v in (-1, 1) for v in d1.values())
    # 別のペア番号では（十分な試行のうちに）別の方向が出る
    assert any(
        spsa_core.deltas(42, k, [P, Q]) != d1 for k in range(101, 130)
    )


def test_perturbed_is_symmetric_and_clamped():
    theta = {"RFP_MULT": 76.0, "LMR_COEF": 2763.0}
    delta = {"RFP_MULT": 1, "LMR_COEF": -1}
    plus = spsa_core.perturbed(theta, [P, Q], delta, 1.0, +1)
    minus = spsa_core.perturbed(theta, [P, Q], delta, 1.0, -1)
    assert plus == {"RFP_MULT": 85, "LMR_COEF": 2513}
    assert minus == {"RFP_MULT": 67, "LMR_COEF": 3013}
    # 可動域の縁ではclampされる
    edge = spsa_core.perturbed({"RFP_MULT": 198.0, "LMR_COEF": 2763.0}, [P, Q], delta, 1.0, +1)
    assert edge["RFP_MULT"] == 200


def test_update_moves_toward_winner():
    theta = {"RFP_MULT": 76.0, "LMR_COEF": 2763.0}
    delta = {"RFP_MULT": 1, "LMR_COEF": -1}
    # θ+側が2連勝（score=+2）ならΔの向きへ動く
    up = spsa_core.update(theta, [P, Q], delta, 1.0, 1.0, 2.0)
    assert up["RFP_MULT"] > theta["RFP_MULT"]
    assert up["LMR_COEF"] < theta["LMR_COEF"]
    # θ−側が勝てば逆へ動き、引き分けだけなら動かない
    down = spsa_core.update(theta, [P, Q], delta, 1.0, 1.0, -2.0)
    assert down["RFP_MULT"] < theta["RFP_MULT"]
    assert spsa_core.update(theta, [P, Q], delta, 1.0, 1.0, 0.0) == theta


def test_update_step_size_matches_formula():
    theta = {"RFP_MULT": 76.0, "LMR_COEF": 2763.0}
    delta = {"RFP_MULT": 1, "LMR_COEF": 1}
    up = spsa_core.update(theta, [P, Q], delta, 2.0, 3.0, 1.0)
    # θ += r_end*r_mult * c_end*c_mult * Δ * score / 2
    assert up["RFP_MULT"] == 76.0 + 0.002 * 3.0 * 9.0 * 2.0 / 2.0


def test_pair_score_counts_candidate_side():
    def rec(candidate, winner):
        return {"candidate": candidate, "winner": winner}

    # candidate（θ+側）の2連勝は+2、1勝1敗は0、引き分けは数えない
    assert spsa_core.pair_score([rec("b", "b"), rec("w", "w")]) == 2.0
    assert spsa_core.pair_score([rec("b", "b"), rec("w", "b")]) == 0.0
    assert spsa_core.pair_score([rec("b", "w"), rec("w", "b")]) == -2.0
    assert spsa_core.pair_score([rec("b", "draw"), rec("w", "w")]) == 1.0
