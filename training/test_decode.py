"""PSVデコードと特徴抽出のテスト。

Rust側の psv dump 出力と突き合わせる。
"""

import sys
import numpy as np

sys.path.insert(0, ".")
from dataset import (
    decode_psv, halfkp_features, effect_features,
    BLACK, WHITE, KING,
    PAWN, LANCE, KNIGHT, SILVER, GOLD, BISHOP, ROOK,
    PRO_PAWN, PRO_LANCE, PRO_KNIGHT, PRO_SILVER, HORSE, DRAGON,
)
from model import FT_IN, EFFECT_IN

PT_SFEN = {
    PAWN: "P", LANCE: "L", KNIGHT: "N", SILVER: "S", GOLD: "G",
    BISHOP: "B", ROOK: "R", KING: "K",
    PRO_PAWN: "+P", PRO_LANCE: "+L", PRO_KNIGHT: "+N", PRO_SILVER: "+S",
    HORSE: "+B", DRAGON: "+R",
}

HAND_ORDER = [ROOK, BISHOP, GOLD, SILVER, KNIGHT, LANCE, PAWN]
HAND_SFEN = {PAWN: "P", LANCE: "L", KNIGHT: "N", SILVER: "S",
             GOLD: "G", BISHOP: "B", ROOK: "R"}


def board_to_sfen(board, turn, hand):
    """Reconstruct SFEN from decoded position (for verification)."""
    parts = []
    for rank in range(9):
        empties = 0
        for file in range(8, -1, -1):
            sq = file * 9 + rank
            piece = board[sq]
            if piece is None:
                empties += 1
                continue
            if empties > 0:
                parts.append(str(empties))
                empties = 0
            color, pt = piece
            s = PT_SFEN[pt]
            if color == WHITE:
                s = s.lower()
            parts.append(s)
        if empties > 0:
            parts.append(str(empties))
        if rank < 8:
            parts.append("/")

    sfen = "".join(parts)
    sfen += " b " if turn == BLACK else " w "

    hand_str = ""
    for c in (BLACK, WHITE):
        for pt in HAND_ORDER:
            n = hand[c][pt]
            if n == 0:
                continue
            s = HAND_SFEN[pt]
            if c == WHITE:
                s = s.lower()
            if n > 1:
                hand_str += str(n)
            hand_str += s
    if not hand_str:
        hand_str = "-"
    sfen += hand_str

    return sfen


def main():
    data = np.fromfile("../data/bench200k.psv", dtype=np.uint8).reshape(-1, 40)
    n_test = min(1000, len(data))

    rust_sfens = [
        "lr7/1n1g3g1/2n1k+BPpl/p1ppspp2/2P1N1sP1/PP1PSP3/2S1P4/2GK1G2P/LR7 w BNPl2p",
        "ln7/2r2gk2/p2gpss1+P/1p3ppp1/3pP2n1/P1P1RSLB1/1PNP1PPP+p/2G2KS1+b/L4G1N1 w Pl",
        "l4gsn1/5rln1/p1+Pp1g2k/4psppp/1p7/2pPS1P1L/PP2PG1P1/8R/LN2KG3 b BSNb3p",
        "lnsgk2nl/6gs1/p1ppppb1p/1r7/6R2/1pP3P2/P2PPP2P/1BG1K4/LNS2GSNL b 3Pp",
        "ln5n1/1r2g1gk1/4p1b1P/p1p1s1pps/1p1p1P2p/2PPP1PP1/PP1S2NK1/1R1BGG1S1/LN6L b lp",
    ]

    print(f"テスト: {n_test}局面")

    # 最初の5局面: SFEN一致テスト
    for i, rust_sfen in enumerate(rust_sfens):
        record = bytes(data[i])
        board, king_sq, turn, hand, score, game_result = decode_psv(record)
        py_sfen = board_to_sfen(board, turn, hand)
        if py_sfen != rust_sfen:
            print(f"SFEN不一致 [{i}]:")
            print(f"  Rust: {rust_sfen}")
            print(f"  Python: {py_sfen}")
            sys.exit(1)

    print("SFEN一致テスト: OK (5/5)")

    # n_test局面: 特徴数・範囲テスト
    errors = 0
    for i in range(n_test):
        record = bytes(data[i])
        try:
            board, king_sq, turn, hand, score, game_result = decode_psv(record)
        except Exception as e:
            print(f"デコードエラー [{i}]: {e}")
            errors += 1
            continue

        stm_feats = halfkp_features(board, king_sq, hand, turn)
        opp_feats = halfkp_features(board, king_sq, hand, 1 - turn)
        ef_feats = effect_features(board, king_sq, turn)

        # HalfKP: 38前後の特徴（盤上38駒-2玉=36 + 手駒）
        if len(stm_feats) < 30 or len(stm_feats) > 60:
            print(f"stm特徴数異常 [{i}]: {len(stm_feats)}")
            errors += 1
        if len(opp_feats) < 30 or len(opp_feats) > 60:
            print(f"opp特徴数異常 [{i}]: {len(opp_feats)}")
            errors += 1

        # 範囲チェック
        if any(f < 0 or f >= FT_IN for f in stm_feats):
            print(f"stm特徴範囲外 [{i}]")
            errors += 1
        if any(f < 0 or f >= FT_IN for f in opp_feats):
            print(f"opp特徴範囲外 [{i}]")
            errors += 1
        if any(f < 0 or f >= EFFECT_IN for f in ef_feats):
            print(f"ef特徴範囲外 [{i}]")
            errors += 1

        # 利き塔: 手番視点で2王の近傍 (最大50特徴、端で18まで減り得る)
        if len(ef_feats) < 10 or len(ef_feats) > 50:
            print(f"ef特徴数異常 [{i}]: {len(ef_feats)}")
            errors += 1

    if errors > 0:
        print(f"エラー: {errors}件")
        sys.exit(1)

    print(f"特徴数・範囲テスト: OK ({n_test}/{n_test})")
    print("全テスト通過")


if __name__ == "__main__":
    main()
