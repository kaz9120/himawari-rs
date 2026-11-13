//! 内部ノードの通常探索。
//!
//! fail-softのalpha-betaと、ノードの後始末（置換表への書き込みと
//! historyの更新）を持つ。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 利きの対数オッズ（`effect_cut_logit`）の、読んだ手の上での平均（1024倍）。
const EFFECT_LOGIT_MEAN: i32 = 224;

impl Worker {
    /// 通常探索。参照実装が `search<PV>` と `search<NonPV>` をテンプレートで
    /// 分けるのに合わせ、ノード種別をconst genericの `PV` で受ける。
    /// 呼び出し側は定数で呼び分ける。
    // 引数はαβ探索の呼び出し規約そのもので、再帰のたびに全部が変わる。
    // 構造体へまとめると1ノードごとに詰め直すことになる
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search<const PV: bool>(
        &mut self,
        mut alpha: Value,
        beta: Value,
        mut depth: u32,
        ply: usize,
        prev: Move,
        pv: &mut Vec<Move>,
        cut_node: bool,
    ) -> Value {
        // 参照実装の不変条件。PVノードはcut_nodeにならない
        debug_assert!(!(PV && cut_node));
        pv.clear();
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

        // mate distance pruning
        alpha = alpha.max(mated_in(ply));
        let beta = beta.min(mate_in(ply + 1));
        if alpha >= beta {
            return alpha;
        }

        // ノードの初期化（yaneuraou-search.cpp:2353-2357）。前の兄弟ノードの
        // 値が残らないよう、手数をここで0へ戻す。王手の有無もここで入れる。
        // TTカット時のhistory更新がcontinuation historyの段数の打ち切りに使う
        let in_check = self.pos.in_check();
        self.stack[ply + STACK_OFFSET].in_check = in_check;
        self.stack[ply + STACK_OFFSET].move_count = 0;
        // 2手先のβカット回数を戻す（yaneuraou-search.cpp:2555）。
        // 自分の次plyは1手前のノードが戻しているので、兄弟をまたいで貯まる
        self.stack[ply + STACK_OFFSET + 2].cutoff_cnt = 0;
        self.stack[ply + STACK_OFFSET].stat_score = 0;
        // 1手前のLMRが削った量（yaneuraou-search.cpp:2552-2553）。
        // 読んだ側が消す。深さの事後補正がこの値を見る
        let prior_reduction = self.stack[ply + STACK_OFFSET - 1].reduction;
        self.stack[ply + STACK_OFFSET - 1].reduction = 0;
        // 前回の反復深化のPVを辿っているか（yaneuraou-search.cpp:2370-2372）。
        // 1手前がPV上にいて、1手前の手が前回PVの同じplyの手と一致するときに
        // 限って真になる。search()はply >= 1でしか呼ばれない
        let follow_pv = self.stack[ply + STACK_OFFSET - 1].follow_pv
            && ply - 1 < self.last_iteration_pv.len()
            && self.stack[ply + STACK_OFFSET - 1].current_move == self.last_iteration_pv[ply - 1];
        self.stack[ply + STACK_OFFSET].follow_pv = follow_pv;
        // 1手前が取った駒と、1手前の移動先（yaneuraou-search.cpp:2355, 2550）。
        // historyの更新条件が繰り返し読む
        let prior_capture = self.pos.captured();
        let prev_move = self.stack[ply + STACK_OFFSET - 1].current_move;
        let prev_sq = if prev_move.is_special() {
            None
        } else {
            Some(prev_move.to())
        };

        // 除外手（singular extension用）。検証探索中はTT手が入る
        let excluded = self.stack[ply + STACK_OFFSET].excluded_move;

        // 置換表
        let key = self.pos.key();
        let tt = self.probe_tt(key, ply, PV, excluded);
        if let Some(v) = self.tt_cutoff(
            &tt,
            ply,
            depth,
            alpha,
            beta,
            PV,
            excluded,
            prior_capture,
            prev_sq,
        ) {
            return v;
        }

        // 置換表の手が駒を取る手か（yaneuraou-search.cpp:2671）。
        // RFPの条件とリダクションの1項が読む
        let tt_capture =
            tt.mv != Move::NONE && !tt.mv.is_drop() && !self.pos.piece_on(tt.mv.to()).is_empty();
        // PVでもcutでもないノード（yaneuraou-search.cpp:2251）。
        // 全手を調べる見込みなのでリダクションを強める。IIRの条件も読む
        let all_node = !(PV || cut_node);

        if depth == 0 {
            // 参照実装はノード種別を引き継ぐ（yaneuraou-search.cpp:2256）
            return self.qsearch::<PV>(alpha, beta, ply);
        }

        // 静的評価。TTのevalを再利用する。
        // rawは補正前（TT保存用）、static_evalはcorrection history補正後。
        let raw_eval = if in_check {
            VALUE_NONE
        } else {
            match &tt.hit {
                Some(d) if Value::from(d.eval) != VALUE_NONE => Value::from(d.eval),
                // TTにevalがなければeval hash経由で計算する
                _ => self.eval_cached(key),
            }
        };
        // correction historyの合成値（yaneuraou-search.cpp:3010）。
        // 静的評価の補正と、LMRのリダクションの減算が同じ値を読む
        let corr_value = self.correction_value(ply);
        // 王手中は評価関数を呼ばず、2手前の静的評価をそのまま写す
        // （yaneuraou-search.cpp:3017）。将棋は王手が続くので、ここを
        // VALUE_NONEにするとimprovingの連鎖が切れて枝刈りが一律に甘くなる
        let static_eval = if in_check {
            self.stack[ply + STACK_OFFSET - 2].static_eval
        } else {
            self.to_corrected_with(raw_eval, corr_value)
        };

        self.stack[ply + STACK_OFFSET].static_eval = static_eval;
        // 置換表の値がこの局面の見積りとしてより適切なら、枝刈り用の評価値へ
        // 採用する（yaneuraou-search.cpp:3084-3087）。下界なら真の値はこれ
        // 以上、上界ならこれ以下と分かっているため。razoringとRFPがこの値を
        // 読み、NMP以降とムーブループはstatic_evalを読む。除外手つき探索と
        // TT不ヒットのときは採用しない（参照実装のStep 6の分岐に対応する）
        let mut eval = static_eval;
        if !in_check && excluded == Move::NONE && tt.hit.is_some() && tt.value != VALUE_NONE {
            let usable = if tt.value > eval {
                matches!(tt.bound, Bound::Lower | Bound::Exact)
            } else {
                matches!(tt.bound, Bound::Upper | Bound::Exact)
            };
            if usable {
                eval = tt.value;
            }
        }
        // 静的評価の差で静かな手のオーダリングを補正する
        // （yaneuraou-search.cpp:3126-3133）。1手前で評価が下がっていれば
        // 1手前の手を良い手とみなして加点する。参照実装は王手中ここへ来ない
        let prev1 = self.stack[ply + STACK_OFFSET - 1];
        if !in_check
            && let Some(prev_sq) = prev_sq
            && !prev1.in_check
            && prior_capture.is_empty()
        {
            let eval_diff = (-(prev1.static_eval + static_eval)).clamp(-214, 171) + 60;
            let them = self.pos.side_to_move().flip();
            self.hist
                .main
                .update(them, prev1.current_move, eval_diff * 10);
            let pc = self.pos.piece_on(prev_sq);
            if tt.hit.is_none()
                && pc.piece_type() != PieceType::PAWN
                && !prev1.current_move.is_promote()
            {
                let slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
                self.shared
                    .hists
                    .pawn
                    .update(slot, pc, prev_sq, eval_diff * 12);
            }
        }

        // ここまでで決まった前提を束ねる。以降の枝刈り・延長・
        // リダクション・終端処理が共通して読む
        let node = NodeInfo {
            ply,
            cut_node,
            all_node,
            in_check,
            follow_pv,
            excluded,
            key,
            raw_eval,
            static_eval,
            eval,
            corr_value,
            prior_reduction,
            prior_capture,
            prev_sq,
            tt,
            tt_capture,
        };

        // 2手前より静的評価が改善しているか（枝刈りの強弱に使う。
        // yaneuraou-search.cpp:3159）。王手中はfalse固定である。
        // 王手中のstatic_evalが2手前の写しなので、連続王手でも連鎖は切れない。
        // 余白の初期値VALUE_NONE（32602）を上回るstatic_evalは存在しないため、
        // ply < 2でも比較だけでfalseになる（参照実装も同じ性質に依存する）
        let mut improving = false;
        if let Some(v) =
            self.prune_before_moves::<PV>(&node, alpha, beta, &mut depth, &mut improving, prev)
        {
            return v;
        }

        // 置換表の下界による簡易ProbCut。探索を伴わない。
        // 除外手つき探索中はスキップする
        let tt_probcut_beta = beta + TT_PROBCUT_MARGIN;
        if excluded == Move::NONE
            && matches!(tt.bound, Bound::Lower | Bound::Exact)
            && tt.depth >= depth.saturating_sub(TT_PROBCUT_DEPTH_SLACK) as i32
            && tt.value >= tt_probcut_beta
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            && tt.value.abs() < VALUE_MATE_IN_MAX_PLY
        {
            return tt_probcut_beta;
        }

        // 延長を積む前の深さ。MovePickerのオーダリングの尺度
        // （yaneuraou-search.cpp:3453）と、TT手のnewDepthの基準
        // （yaneuraou-search.cpp:3556）がこの値を読む。参照実装はどちらも
        // depthを増やす前に決まるので、singularでdepthが増えても動かない
        let depth_pre_singular = depth as i32;
        // TT手に与える延長。Noneは判定に入らなかったことを表す
        let singular_ext = match self.singular_extension::<PV>(&node, beta, &mut depth, prev) {
            Ok(e) => e,
            // multi-cutと打ち切りはこのノードごと抜ける
            Err(v) => return v,
        };

        // ply別の再利用バッファを借りる。同一plyへ再帰する
        // singular検証・NMP検証はここへ来る前に終わっているので重ならない
        let buf = std::mem::take(&mut self.node_bufs[ply].moves);
        let mut picker = MovePicker::new(&self.pos, tt.mv, depth_pre_singular, ply, buf);
        // continuation historyの面（1手前から6手前まで）
        let cont = self.cont_bases(ply);
        // 最善にならなかった手を良い順に覚える（yaneuraou-search.cpp:2343-2344）
        let mut quiets_searched = std::mem::take(&mut self.node_bufs[ply].quiets);
        let mut captures_searched = std::mem::take(&mut self.node_bufs[ply].captures);
        // 子のPVの置き場。子の `search` が冒頭でclearするので、
        // 前のノードの中身が残っていても読まれない
        let mut child_pv = std::mem::take(&mut self.node_bufs[ply].child_pv);
        quiets_searched.clear();
        captures_searched.clear();

        // バッファの返却を一本化するため、ここから先の復帰点をこのブロックに
        // まとめる。途中で `return` すると返却が漏れる
        let node_value = 'node: {
            let mut best = -VALUE_INFINITE;
            let mut best_move = Move::NONE;
            let mut best_move_is_capture = false;
            let mut count = 0u32;

            while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
                // 除外手はスキップ（singular検証探索）。通常はexcluded==NONE
                if m == excluded {
                    continue;
                }
                if !self.pos.is_legal_picked(m, picker.last_from_tt()) {
                    continue;
                }
                // 枝刈りの判定の間に、子が引く表の読み込みを進めさせる
                self.prefetch_child(m);
                count += 1;
                // 1手先のノードが更新条件で読む（yaneuraou-search.cpp:3520）
                self.stack[ply + STACK_OFFSET].move_count = count;
                let is_capture = !m.is_drop() && !self.pos.piece_on(m.to()).is_empty();
                let gives_check = self.pos.gives_check(m);

                // 延長（yaneuraou-search.cpp:3543, 3872）。singularの判定に入った
                // TT手はその結果を使う。王手延長は参照実装が持たない
                // 本エンジンの機能なので残すが、singularの判定が付いた手では
                // singular側を採る（多段化した延長量を王手延長で潰さない）。
                // 参照実装は延長をムーブループの枝刈りの後で加えるので、枝刈りの
                // 尺度（lmr_depth）はこの値でなく `depth - 1` を基準にする
                let (extension, base_depth) = match singular_ext {
                    // singularの判定が付いたTT手。参照実装はムーブループの中で
                    // `newDepth = depth - 1` を先に決め、そのあとdepthを増やすので、
                    // 増えた1手はTT手自身には乗らない
                    // （yaneuraou-search.cpp:3556, 3788）
                    Some(e) if m == tt.mv => (e, depth_pre_singular),
                    // 王手の延長は根の深さの2倍の手数までにする。上限が
                    // ないと王手と応手の連鎖で深さが減らず、MAX_PLY付近でノードが膨らむ
                    _ => (
                        i32::from(gives_check && (ply as u32) < 2 * self.root_depth),
                        depth as i32,
                    ),
                };
                let mut new_depth = base_depth - 1 + extension;

                // LMRのリダクション量（1024倍の固定小数）。枝刈りの尺度
                // （lmr_depth）と実際の浅い探索で同じ値を使う
                let delta = beta - alpha;
                let mut r = self.reduction(improving, depth, count, delta);
                // 項1: ttPvノードは削る（yaneuraou-search.cpp:3573-3574）。
                // 枝刈りの尺度に入るのはここまでで、残りはdo_moveの側で足す
                if self.stack[ply + STACK_OFFSET].tt_pv {
                    r += 1013;
                }
                // Step 14: 浅い深さでの枝刈り（yaneuraou-search.cpp:3586-3698）
                if self.prune_shallow::<PV>(
                    &node,
                    &mut picker,
                    &mut best,
                    m,
                    alpha,
                    depth,
                    count,
                    r,
                    improving,
                    is_capture,
                    gives_check,
                    best_move,
                    &cont,
                ) {
                    continue;
                }

                // リダクションの加減算（yaneuraou-search.cpp:3879-3941）
                let mut r = self.reduction_amount::<PV>(&node, m, &cont, r, alpha, depth, count);
                // 利きの項。βカットの見込みが平均より高い手ほど削る量を減らす。
                // 対数オッズ1を1手に換算し、平均（224）を引いて削る量の総和を保つ
                if count > 1 && m != tt.mv {
                    r -= self.pos.effect_cut_logit(m, gives_check, is_capture) - EFFECT_LOGIT_MEAN;
                }

                self.set_current_move(ply, m, is_capture);
                self.make_move_prefetched(m);

                let mut value = -VALUE_INFINITE;
                if depth >= 2 && count > 1 {
                    // LMR（yaneuraou-search.cpp:3954-4010）。参照実装の発動条件は
                    // 深さと手数だけで、取る手も王手する手も対象にする。
                    // リダクションが負なら `new_depth + 2` まで深く読む
                    let d = (new_depth - r / 1024).min(new_depth + 2).max(1) + i32::from(PV);
                    self.stack[ply + STACK_OFFSET].reduction = new_depth - d;
                    value = -self.search::<false>(
                        -alpha - 1,
                        -alpha,
                        d as u32,
                        ply + 1,
                        m,
                        &mut child_pv,
                        true,
                    );
                    self.stack[ply + STACK_OFFSET].reduction = 0;
                    if value > alpha {
                        // 減深探索の結果で読み直す深さを調整する
                        // （yaneuraou-search.cpp:3997-4000）。十分良ければ深く、
                        // 十分悪ければ浅くする
                        let do_deeper = d < new_depth && value > best + 48;
                        let do_shallower = value < best + 9;
                        new_depth += i32::from(do_deeper) - i32::from(do_shallower);
                        if new_depth > d && !self.stopped() {
                            value = -self.search::<false>(
                                -alpha - 1,
                                -alpha,
                                new_depth.max(0) as u32,
                                ply + 1,
                                m,
                                &mut child_pv,
                                !cut_node,
                            );
                        }
                        // LMR後のcontinuation history更新
                        // （yaneuraou-search.cpp:4008）
                        self.update_continuation_histories(ply, m.piece_after(), m.to(), 1426);
                    }
                } else if !PV || count > 1 {
                    // LMRを省いたときの調整（yaneuraou-search.cpp:4017-4030）。
                    // 項12: TT手がなければ削る。削る量が大きければ深さを落とす
                    if tt.mv == Move::NONE {
                        r += 1057;
                    }
                    let d = new_depth - i32::from(r > 4628) - i32::from(r > 5772 && new_depth > 2);
                    value = -self.search::<false>(
                        -alpha - 1,
                        -alpha,
                        d.max(0) as u32,
                        ply + 1,
                        m,
                        &mut child_pv,
                        !cut_node,
                    );
                }
                // PVノードは第1手とfail highの後だけ全窓で読み直す
                // （yaneuraou-search.cpp:4043-4061）
                if PV && (count == 1 || value > alpha) && !self.stopped() {
                    // 静止探索へ直行する手前で、TT手だけは1手残す
                    // （yaneuraou-search.cpp:4053-4057）。負の延長でnew_depthが
                    // 0以下になったTT手をqsearchへ落とすと、詰みの発見が鈍る
                    if m == tt.mv
                        && ((tt.value != VALUE_NONE
                            && tt.value.abs() >= VALUE_MATE_IN_MAX_PLY
                            && tt.depth > 0)
                            || tt.depth > 1)
                    {
                        new_depth = new_depth.max(1);
                    }
                    value = -self.search::<true>(
                        -beta,
                        -alpha,
                        new_depth.max(0) as u32,
                        ply + 1,
                        m,
                        &mut child_pv,
                        false,
                    );
                }
                self.unmake_move(m);
                if self.stopped() {
                    break 'node VALUE_ZERO;
                }

                if value > best {
                    best = value;
                    if value > alpha {
                        best_move = m;
                        best_move_is_capture = is_capture;
                        if PV {
                            pv.clear();
                            pv.push(m);
                            pv.extend_from_slice(&child_pv);
                        }
                        if value >= beta {
                            // 次plyのfail highの多さをリダクションへ渡す
                            // （yaneuraou-search.cpp:4214）。2手以上延長した手の
                            // カットは数えない（延長は最大+3まである）
                            if extension < 2 || PV {
                                self.stack[ply + STACK_OFFSET].cutoff_cnt += 1;
                            }
                            break;
                        }
                        // alphaを更新できたので、残りの手を浅く読む
                        // （yaneuraou-search.cpp:4228-4229）。決着スコアのときは
                        // 深さを保って読み切る
                        if depth > 2 && depth < 14 && value.abs() < VALUE_MATE_IN_MAX_PLY {
                            depth -= 2;
                        }
                        alpha = value;
                    }
                }

                // 最善でなかった手を、あとでmalusを配るために覚えておく
                // （yaneuraou-search.cpp:4246-4256）。上限は32手
                if m != best_move && count <= SEARCHED_LIST_CAPACITY {
                    if is_capture {
                        captures_searched.push(m);
                    } else {
                        quiets_searched.push(m);
                    }
                }
            }

            if count == 0 {
                // 除外手つき探索（singularの検証）では、TT手を除いたせいで手が
                // 尽きただけなのでfail lowのスコアを返す（yaneuraou-search.cpp:4295）。
                // 詰みの値を返すと、TT手が唯一の合法手であるノードの検証値が
                // 常に最小になり、多段延長が必ず最大まで積む
                if excluded != Move::NONE {
                    break 'node alpha;
                }
                // 合法手なし = 詰み（将棋はステイルメイトも負け）
                break 'node mated_in(ply);
            }

            self.finalize_node::<PV>(
                &node,
                depth,
                alpha,
                beta,
                best,
                best_move,
                best_move_is_capture,
                &quiets_searched,
                &captures_searched,
            );
            best
        };

        // 借りたバッファを戻す。容量が残るので次の同じplyでは確保が起きない
        self.node_bufs[ply].moves = picker.into_buf();
        self.node_bufs[ply].quiets = quiets_searched;
        self.node_bufs[ply].captures = captures_searched;
        self.node_bufs[ply].child_pv = child_pv;
        node_value
    }

    /// ノードの終端処理（yaneuraou-search.cpp:4299-4418）。指し手の統計、
    /// 1手前のttPvの引き継ぎ、TT store、correction historyの更新をこの順に行う。
    ///
    /// `quiets_searched` と `captures_searched` は、このノードで調べたが
    /// 最善にならなかった手を良い順に並べたもの。
    // 入口で決まる値はNodeInfoへまとめてある。残りは手のループを回し終えて
    // 初めて確定する結果なので、NodeInfoへは入れられない
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finalize_node<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        depth: u32,
        alpha: Value,
        beta: Value,
        best: Value,
        best_move: Move,
        best_move_is_capture: bool,
        quiets_searched: &[Move],
        captures_searched: &[Move],
    ) {
        let &NodeInfo {
            ply,
            in_check,
            excluded,
            key,
            raw_eval,
            static_eval,
            prior_capture,
            prev_sq,
            tt,
            ..
        } = node;
        // 指し手の統計を更新する（yaneuraou-search.cpp:4299-4356）。
        // βカットしていなくても、alphaを更新した手があれば更新する
        if best_move != Move::NONE {
            self.update_all_stats(
                ply,
                depth,
                best_move,
                tt.mv,
                prev_sq,
                prior_capture,
                quiets_searched,
                captures_searched,
            );
            // 非PVノードのbestMove確定時の更新（yaneuraou-search.cpp:4308）。
            // もう1か所、multi-cutが減点する（yaneuraou-search.cpp:3819）
            if !PV {
                self.hist
                    .tt_move
                    .update(if best_move == tt.mv { 805 } else { -787 });
            }
        } else if let Some(prev_sq) = prev_sq
            && prior_capture.is_empty()
        {
            // fail lowを引き起こした1手前の静かな手への加点
            // （yaneuraou-search.cpp:4320-4341）
            let prev1 = self.stack[ply + STACK_OFFSET - 1];
            let mut bonus_scale = -232;
            bonus_scale -= prev1.stat_score / 108;
            bonus_scale += (59 * depth as i32).min(454);
            bonus_scale += 169 * i32::from(prev1.move_count > 8);
            bonus_scale += 145 * i32::from(!in_check && best <= static_eval - 110);
            bonus_scale += 154 * i32::from(!prev1.in_check && best <= -prev1.static_eval - 73);
            let bonus_scale = bonus_scale.max(0);
            let scaled = (135 * depth as i32 - 80).min(1400) * bonus_scale;

            let pc = self.pos.piece_on(prev_sq);
            self.update_continuation_histories(ply - 1, pc, prev_sq, scaled * 221 / 16384);
            let them = self.pos.side_to_move().flip();
            self.hist
                .main
                .update(them, prev1.current_move, scaled * 235 / 32768);
            if pc.piece_type() != PieceType::PAWN && !prev1.current_move.is_promote() {
                let slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
                self.shared
                    .hists
                    .pawn
                    .update(slot, pc, prev_sq, scaled * 290 / 8192);
            }
        } else if let Some(prev_sq) = prev_sq {
            // fail lowを引き起こした1手前の取る手への加点
            // （yaneuraou-search.cpp:4346-4351）
            self.hist.capture.update(
                self.pos.piece_on(prev_sq),
                prev_sq,
                prior_capture.piece_type(),
                1018,
            );
        }

        // 良い手が見つからなかったなら1手前のttPvを引き継ぐ
        // （yaneuraou-search.cpp:4367-4368）。1手前が探索木に入れた変化なら、
        // この局面も探索木へ加える
        if best <= alpha {
            self.stack[ply + STACK_OFFSET].tt_pv =
                self.stack[ply + STACK_OFFSET].tt_pv || self.stack[ply + STACK_OFFSET - 1].tt_pv;
        }

        // 除外手つき探索中はTT storeをしない。
        // 検証専用の結果でこのキーの本体を汚さない
        if excluded == Move::NONE {
            let bound = if best >= beta {
                Bound::Lower
            } else if PV && best_move != Move::NONE {
                Bound::Exact
            } else {
                Bound::Upper
            };
            self.shared.tt.store(
                key,
                best_move.to_move16(),
                value_to_tt(best, ply),
                raw_eval as i16,
                (depth as i32 + TT_DEPTH_OFFSET).min(255) as u8,
                bound,
                // 参照実装はis_pvではなくttPvを書き戻す
                // （yaneuraou-search.cpp:4397）
                self.stack[ply + STACK_OFFSET].tt_pv,
            );
        }

        // correction history更新。最善手が取る手でなく、
        // 誤差の向きが最善手の有無と一致するときだけ蓄積する
        // （yaneuraou-search.cpp:4410-4418）。参照実装は除外手つき探索でも
        // 更新するので、TT storeと違いここは条件に入れない
        if !in_check
            && !(best_move != Move::NONE && best_move_is_capture)
            && (best > static_eval) == (best_move != Move::NONE)
        {
            let w = if best_move != Move::NONE { 12 } else { 17 };
            // クランプ幅は値域の1/4（history.h:30のCORRECTION_HISTORY_LIMIT=1024）
            let bonus = ((best - static_eval) * depth as i32 * w / 128).clamp(-256, 256);
            self.update_correction_history(ply, 1069 * bonus / 1024);
        }
    }
}
