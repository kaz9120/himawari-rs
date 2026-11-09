//! historyとcorrection historyの更新。

#[allow(clippy::wildcard_imports)]
use super::*;

impl Worker {
    /// correction historyを6要素まとめて更新する。
    /// 出典はやねうら王の `update_correction_history()`
    /// （yaneuraou-search.cpp:748-771）。系統ごとに重みが違う。
    pub(super) fn update_correction_history(&mut self, ply: usize, bonus: i32) {
        self.shared.hists.corr.update_all(&self.pos, bonus);
        let prev1 = self.stack[ply + STACK_OFFSET - 1].current_move;
        if !prev1.is_special() {
            let to = prev1.to();
            let pc = self.pos.piece_on(to);
            let base2 = self.stack[ply + STACK_OFFSET - 2].cont_corr_base;
            let base4 = self.stack[ply + STACK_OFFSET - 4].cont_corr_base;
            self.hist.corr_cont.update(base2, pc, to, bonus * 126 / 128);
            self.hist.corr_cont.update(base4, pc, to, bonus * 63 / 128);
        }
    }

    /// continuation historyへ段ごとの重みでbonusを配る。
    /// 出典はやねうら王の `update_continuation_histories()`
    /// （yaneuraou-search.cpp:5384-5398）。王手中は2手前までで打ち切る。
    pub(super) fn update_continuation_histories(
        &mut self,
        ply: usize,
        pc: Piece,
        to: Square,
        bonus: i32,
    ) {
        /// 何手前の面にどれだけ配るか（yaneuraou-search.cpp:5385-5386）。
        const CONTHIST_BONUSES: [(usize, i32); 6] =
            [(1, 1157), (2, 648), (3, 288), (4, 576), (5, 140), (6, 441)];
        let in_check = self.stack[ply + STACK_OFFSET].in_check;
        for (i, weight) in CONTHIST_BONUSES {
            if in_check && i > 2 {
                break;
            }
            let e = self.stack[ply + STACK_OFFSET - i];
            if !e.current_move.is_special() {
                let b = bonus * weight / 1024 + 88 * i32::from(i < 2);
                self.hist.cont.update(e.cont_base, pc, to, b);
            }
        }
    }

    /// 静かな手1つのhistoryを更新する。
    /// 出典はやねうら王の `update_quiet_histories()`
    /// （yaneuraou-search.cpp:5408-5422）。
    pub(super) fn update_quiet_histories(&mut self, ply: usize, m: Move, bonus: i32) {
        self.hist.main.update(self.pos.side_to_move(), m, bonus);
        if ply < LOW_PLY_HISTORY_SIZE {
            self.hist.low_ply.update(ply, m, bonus * 761 / 1024);
        }
        self.update_continuation_histories(ply, m.piece_after(), m.to(), bonus * 955 / 1024);
        let slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
        let scaled = bonus * if bonus > 0 { 850 } else { 550 } / 1024;
        self.shared
            .hists
            .pawn
            .update(slot, m.piece_after(), m.to(), scaled);
    }

    /// 統計情報一式を更新する。bestMoveが確定したノードの
    /// 終端で呼ぶ。出典はやねうら王の `update_all_stats()`
    /// （yaneuraou-search.cpp:5293-5367）。
    ///
    /// `quiets_searched` と `captures_searched` は、このノードで調べたが
    /// 最善にならなかった手を良い順に並べたもの。
    // 引数は出典の関数（yaneuraou-search.cpp:5293-5367）と同じ役の値である。
    // 参照実装と読み比べるために並びを変えない。呼び出しはfinalize_nodeの1か所
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_all_stats(
        &mut self,
        ply: usize,
        depth: u32,
        best_move: Move,
        tt_move: Move,
        prev_sq: Option<Square>,
        prior_capture: Piece,
        quiets_searched: &[Move],
        captures_searched: &[Move],
    ) {
        // bonus式（yaneuraou-search.cpp:5307-5309）。第3項は1手前の
        // statScore、つまりこのノードへ来た手の履歴の強さである
        let bonus = (128 * depth as i32 - 77).min(1529)
            + 353 * i32::from(best_move == tt_move)
            + self.stack[ply + STACK_OFFSET - 1].stat_score / 32;
        let malus = (882 * depth as i32 - 204).min(2122);

        let to = best_move.to();
        let captured = self.pos.piece_on(to);
        if best_move.is_drop() || captured.is_empty() {
            self.update_quiet_histories(ply, best_move, bonus * 806 / 1024);
            // 最善でなかった静かな手へmalusを配る。後ろの手ほど軽くする
            // （yaneuraou-search.cpp:5318-5326）
            let mut actual_malus = malus * 1113 / 1024;
            for &q in quiets_searched {
                actual_malus = actual_malus * 977 / 1024;
                self.update_quiet_histories(ply, q, -actual_malus);
            }
        } else {
            self.hist.capture.update(
                best_move.piece_after(),
                to,
                captured.piece_type(),
                bonus * 1286 / 1024,
            );
        }

        // 1手前が置換表の手でない早い静かな手で、それが反証されたときの
        // 追加ペナルティ（yaneuraou-search.cpp:5344-5345）
        if let Some(prev_sq) = prev_sq {
            let prev = self.stack[ply + STACK_OFFSET - 1];
            if prev.move_count == 1 + u32::from(prev.tt_hit) && prior_capture.is_empty() {
                let pc = self.pos.piece_on(prev_sq);
                self.update_continuation_histories(ply - 1, pc, prev_sq, -malus * 616 / 1024);
            }
        }

        // 最善でなかった取る手へmalusを配る（yaneuraou-search.cpp:5359-5364）
        for &c in captures_searched {
            let to = c.to();
            self.hist.capture.update(
                c.piece_after(),
                to,
                self.pos.piece_on(to).piece_type(),
                -malus * 1559 / 1024,
            );
        }
    }
}
