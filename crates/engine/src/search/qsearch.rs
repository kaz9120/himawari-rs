//! 静止探索。

#[allow(clippy::wildcard_imports)]
use super::*;

impl Worker {
    /// 静止探索。出典はやねうら王の `qsearch()`
    /// （yaneuraou-search.cpp:4441-5145）。参照実装は `qsearch<PV>` と
    /// `qsearch<NonPV>` をテンプレートで分けるので、const genericの `PV` で
    /// 受ける。
    pub(super) fn qsearch<const PV: bool>(
        &mut self,
        mut alpha: Value,
        beta: Value,
        ply: usize,
    ) -> Value {
        if self.stopped() {
            return VALUE_ZERO;
        }
        self.sel_depth = self.sel_depth.max(ply as u32);
        self.nodes += 1;
        self.check_limits();
        if ply >= MAX_PLY {
            return self.evaluator.evaluate(&self.pos);
        }
        if let Some(v) = self.terminal_value(ply) {
            return v;
        }

        // Step 3. 置換表probe（yaneuraou-search.cpp:4645-4685）
        let key = self.pos.key();
        let tt_hit = self.shared.tt.probe(key);
        // 参照実装はqsearchでもStackへ記録する（yaneuraou-search.cpp:4648）
        self.stack[ply + STACK_OFFSET].tt_hit = tt_hit.is_some();
        // 置換表にPVノードとして記録された値か（yaneuraou-search.cpp:4657）。
        // 末尾のstoreへそのまま書き戻す
        let pv_hit = tt_hit.as_ref().is_some_and(|d| d.pv);
        let mut tt_move = Move::NONE;
        let mut tt_eval = VALUE_NONE;
        let mut tt_value = VALUE_NONE;
        let mut tt_bound = Bound::None;
        if let Some(data) = &tt_hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt_move = m;
            }
            tt_eval = Value::from(data.eval);
            tt_value = value_from_tt(data.value, ply);
            tt_bound = data.bound;
            // TTカットは非PVノードだけで行う（yaneuraou-search.cpp:4670-4679）。
            // PVノードでは前回evaluateした値が使えるのでカットしない。
            // DEPTH_UNSEARCHEDで書かれたstand patの記録はここを通らない
            if !PV
                && data.depth >= TT_DEPTH_QS
                && tt_value != VALUE_NONE
                // 原典の `bound & (value >= beta ? LOWER : UPPER)` をそのまま写す
                && if tt_value >= beta {
                    matches!(tt_bound, Bound::Lower | Bound::Exact)
                } else {
                    matches!(tt_bound, Bound::Upper | Bound::Exact)
                }
            {
                return tt_value;
            }
        }

        let in_check = self.pos.in_check();
        // continuation historyの面を決める材料
        self.stack[ply + STACK_OFFSET].in_check = in_check;

        // Step 4. 静的評価（yaneuraou-search.cpp:4713-4837）。
        // rawは補正前でTT storeのeval欄へそのまま入る。王手中は定義しない
        let mut raw_eval = VALUE_NONE;
        let mut best = -VALUE_INFINITE;
        // 静止探索のfutilityの基準。王手中は定義しない
        let mut futility_base = -VALUE_INFINITE;
        if !in_check {
            let corr_value = self.correction_value(ply);
            // 1手詰め判定（yaneuraou-search.cpp:4756-4784）。
            // 置換表にヒットしたときはすでに調べたはずなので、ミスのときだけ
            // 呼ぶ。王手中も呼ばない。評価関数の呼び出しより手前に置くので、
            // 詰みがあるときはevaluate()を省ける
            if tt_hit.is_none()
                && let Some(m) = self.pos.mate_1ply()
            {
                // 次のノードで（指し手がなくなって）詰むという解釈
                let mate = mate_in(ply + 1);
                // 原典はmate_in()の値をvalue_to_tt()に通さず、そのまま書く
                // （yaneuraou-search.cpp:4777と、同じ処理の説明がある2911-2914）。
                // eval欄は評価関数を呼ぶ前なのでVALUE_NONEである
                self.shared.tt.store(
                    key,
                    m.to_move16(),
                    mate as i16,
                    VALUE_NONE as i16,
                    TT_DEPTH_QS,
                    Bound::Exact,
                    self.stack[ply + STACK_OFFSET].tt_pv,
                );
                return mate;
            }
            // TTのeval欄が空なら評価関数を呼ぶ（yaneuraou-search.cpp:4729-4731）
            raw_eval = if tt_eval != VALUE_NONE {
                tt_eval
            } else {
                self.eval_cached(key)
            };
            // 参照実装は静止探索でもstaticEvalをStackへ記録する
            // （yaneuraou-search.cpp:4729, 4790）。親のimprovingがこの値を読む
            let stand = self.to_corrected_with(raw_eval, corr_value);
            self.stack[ply + STACK_OFFSET].static_eval = stand;
            best = stand;
            // TTの値のほうがこの局面の見積りとして良ければ採用する
            // （yaneuraou-search.cpp:4743-4745）。詰みスコアは動かさない
            if tt_hit.is_some()
                && tt_value != VALUE_NONE
                && tt_value.abs() < VALUE_MATE_IN_MAX_PLY
                // 原典の `bound & (value > bestValue ? LOWER : UPPER)` をそのまま写す
                && if tt_value > best {
                    matches!(tt_bound, Bound::Lower | Bound::Exact)
                } else {
                    matches!(tt_bound, Bound::Upper | Bound::Exact)
                }
            {
                best = tt_value;
            }
            // stand pat（yaneuraou-search.cpp:4813-4823）
            if best >= beta {
                // 返す値をβへ半分寄せる（yaneuraou-search.cpp:4815-4817）。
                // βを大きく超えたstand patをそのまま返さず、見積りの甘さを削る。
                // 参照実装はここで決着スコアかを確かめるが、本エンジンでは
                // bestが詰み圏に入らない。補正後の評価値はクランプ済みで、
                // TTの値も詰み圏の外のものしか採らない。詰みは1手詰め判定が
                // 手前で返す（Issue #698）
                debug_assert!(best.abs() < VALUE_MATE_IN_MAX_PLY);
                best = (best + beta) / 2;
                // TTにヒットしていなければ、探索を伴わない値として書き出す。
                // depthはDEPTH_UNSEARCHEDなのでTTカットには使われない
                if tt_hit.is_none() {
                    self.shared.tt.store(
                        key,
                        Move::NONE.to_move16(),
                        value_to_tt(best, ply),
                        raw_eval as i16,
                        TT_DEPTH_UNSEARCHED,
                        Bound::Lower,
                        false,
                    );
                }
                return best;
            }
            if best > alpha {
                alpha = best;
            }
            // 基準はTT値で上書きする前のstaticEvalである
            // （yaneuraou-search.cpp:4836）
            futility_base = stand + QS_FUTILITY_MARGIN();
        }
        // 1手前の移動先（取り返しをfutilityの対象から外す）。
        // 余白の初期値はMove::NONEなので、ply 0でもNoneになる
        let prev_move = self.stack[ply + STACK_OFFSET - 1].current_move;
        let prev_sq = if prev_move != Move::NONE && !prev_move.is_special() {
            Some(prev_move.to())
        } else {
            None
        };

        // 参照実装は静止探索で取る手（歩成を含む）だけを生成する
        // （movepick.cpp:69）。静かな王手は生成しない
        // ply別の再利用バッファを借りる
        let buf = std::mem::take(&mut self.node_bufs[ply].moves);
        let mut picker = MovePicker::new(&self.pos, tt_move, 0, ply, buf);
        let cont = self.cont_bases(ply);
        // バッファの返却を一本化するため、復帰点をこのブロックにまとめる
        let node_value = 'node: {
            let mut count = 0u32;
            let mut best_move = Move::NONE;
            while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
                if !self.pos.is_legal_picked(m, picker.last_from_tt()) {
                    continue;
                }
                self.prefetch_child(m);
                count += 1;
                // capture_stage(m) は将棋版では単なるcapture(m)（position.h:1317-1320）
                let capture = !m.is_drop() && !self.pos.piece_on(m.to()).is_empty();

                // Step 6. 枝刈り（yaneuraou-search.cpp:4930-4991）。
                // bestValueが負け側の決着スコアの間は何も刈らない。詰みを逃れる
                // 手を探している最中だからである
                if best > VALUE_MATED_IN_MAX_PLY {
                    // このしきい値で通ったSEEは、これ以下のしきい値でも通る
                    let mut see_passed_at = None;
                    // futility: 王手をかけず取り返しでもない手を、
                    // 取る駒の価値を足してもalphaへ届かないなら捨てる
                    if futility_base > VALUE_MATED_IN_MAX_PLY
                        && Some(m.to()) != prev_sq
                        && !self.pos.gives_check(m)
                    {
                        if count > QS_MOVECOUNT_LIMIT {
                            continue;
                        }
                        let captured = self.pos.piece_on(m.to());
                        let gain = if captured.is_empty() {
                            VALUE_ZERO
                        } else {
                            himawari_core::piece_value(captured.piece_type())
                        };
                        // 捨てる前にbestを引き上げる（yaneuraou-search.cpp:4950-4954,
                        // 4965-4969）。fail-softの下限を正しく報告するためである。
                        // 検討モード（MultiPV>1）だけは抑える。ライン確定ごとに出力し、
                        // 確定後のソートを持たないので、窓に依存する値を入れると
                        // ライン間のスコア順序が崩れる
                        let raise = self.multi_pv == 1;
                        let futility_value = futility_base + gain;
                        if futility_value <= alpha {
                            if raise {
                                best = best.max(futility_value);
                            }
                            continue;
                        }
                        let threshold = alpha - futility_base;
                        if !self.pos.see_ge(m, threshold) {
                            if raise {
                                best = best.max(alpha.min(futility_base));
                            }
                            continue;
                        }
                        see_passed_at = Some(threshold);
                    }

                    // 取る手でない手は一律に捨てる（yaneuraou-search.cpp:4975-4976）
                    if !capture {
                        continue;
                    }
                    // SEEが十分悪い手は探索しない（yaneuraou-search.cpp:4989-4990）。
                    // 無駄な王手ラッシュを抑える。歩損は許す下限である
                    let margin = QS_SEE_MARGIN();
                    if !see_passed_at.is_some_and(|th| th >= margin) && !self.pos.see_ge(m, margin)
                    {
                        continue;
                    }
                }

                self.set_current_move(ply, m, capture);
                self.make_move_prefetched(m);
                let value = -self.qsearch::<PV>(-beta, -alpha, ply + 1);
                self.unmake_move(m);
                if self.stopped() {
                    break 'node VALUE_ZERO;
                }
                if value > best {
                    best = value;
                    if value > alpha {
                        best_move = m;
                        alpha = value;
                        if value >= beta {
                            break;
                        }
                    }
                }
            }
            // Step 9. 詰みの確認（yaneuraou-search.cpp:5054-5070）。
            // 王手中に合法手が尽きたら詰み。参照実装は置換表へ書かずに返す。
            // 再訪問の確率が極めて低く、置換表を汚すだけだからである
            if in_check && count == 0 {
                break 'node mated_in(ply);
            }

            // 決着スコアでなければ、返す値をβへ半分寄せる
            // （yaneuraou-search.cpp:5093-5094）。stand patと違い、ここの
            // bestは子の静止探索の戻り値なので詰みスコアを取りうる
            if best.abs() < VALUE_MATE_IN_MAX_PLY && best > beta {
                best = (best + beta) / 2;
            }

            // 置換表store（yaneuraou-search.cpp:5124-5126）。深さはDEPTH_QS固定。
            // 静止探索の結果は信用ならないのでBOUND_EXACTは書かない
            self.shared.tt.store(
                key,
                best_move.to_move16(),
                value_to_tt(best, ply),
                raw_eval as i16,
                TT_DEPTH_QS,
                if best >= beta {
                    Bound::Lower
                } else {
                    Bound::Upper
                },
                pv_hit,
            );
            best
        };

        // 借りたバッファを戻す
        self.node_bufs[ply].moves = picker.into_buf();
        node_value
    }
}
