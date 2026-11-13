//! 枝刈り・延長・リダクションの判定。
//!
//! 指し手ループの前に行う枝刈り（RFP・NMP・ProbCutなど）、手ごとの
//! 浅い枝刈り、singular extension、LMRのリダクション量を持つ。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 3手詰めを調べる残り深さの上限。これより深いノードは探索そのものが3手詰めを読む。
const MATE3_MAX_DEPTH: u32 = 4;
/// 3手詰めを調べる、相手玉の逃げ場の数の上限。利きの配列から数えるので安い。
const MATE3_MAX_ESCAPES: u32 = 1;

impl Worker {
    /// 短い詰みを探し、初手と詰みの値を返す。1手詰めは常に、3手詰めは浅いノードで
    /// 相手玉の逃げ場が少ないときだけ調べる。
    pub(super) fn find_short_mate(&mut self, depth: u32, ply: usize) -> Option<(Move, Value)> {
        if let Some(m) = self.pos.mate_1ply() {
            return Some((m, mate_in(ply + 1)));
        }
        if depth <= MATE3_MAX_DEPTH
            && self.pos.enemy_king_escapes() <= MATE3_MAX_ESCAPES
            && let Some(m) = self.pos.mate_3ply()
        {
            return Some((m, mate_in(ply + 3)));
        }
        None
    }

    /// LMRのリダクション量の加減算（yaneuraou-search.cpp:3879-3941）。
    /// 参照実装はdo_moveの後に置くが、読む材料は進める前の局面で決まるので
    /// ここでまとめる。項1（ttPv）は枝刈りの尺度にも入るため呼び出し側にある。
    ///
    /// 項10の途中で、その手の履歴の強さをStackへ控える。子のhistory更新量も
    /// この値を読むので、do_moveの前に測る必要がある
    // ノードで決まる値はNodeInfoへまとめてある。残りは手ごとに変わる値で、
    // 1手ずつ呼ぶたびに差し替わる
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reduction_amount<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        m: Move,
        cont: &[usize; 6],
        mut r: i32,
        alpha: Value,
        depth: u32,
        count: u32,
    ) -> i32 {
        let &NodeInfo {
            ply,
            cut_node,
            all_node,
            corr_value,
            tt,
            tt_capture,
            ..
        } = node;
        // 項2: ttPvノードは大きく戻す。TTの値がalphaを超える、TTの
        // 深さが足りている、といった手掛かりがあるほど戻す
        if self.stack[ply + STACK_OFFSET].tt_pv {
            r -= 2819
                + i32::from(PV) * 973
                + i32::from(tt.value > alpha) * 905
                + i32::from(tt.depth >= depth as i32) * (935 + i32::from(cut_node) * 959);
        }
        // 項3: 他の調整を補正する基準オフセット
        r += 691;
        // 項4: 手数が進むほど戻す
        r -= count as i32 * 65;
        // 項5: correction historyの補正が大きい局面は戻す
        r -= corr_value.abs() / 25600;
        // 項6: cutNodeは削る。TT手がなければさらに削る
        if cut_node {
            r += 3611 + 985 * i32::from(tt.mv == Move::NONE);
        }
        // 項7: TT手が駒を取る手なら削る
        if tt_capture {
            r += 1054;
        }
        // 項8: 次plyでfail highが多いなら削る
        let child_cutoffs = self.stack[ply + STACK_OFFSET + 1].cutoff_cnt;
        if child_cutoffs > 1 {
            r += 251 + 1124 * i32::from(child_cutoffs > 2) + 1042 * i32::from(all_node);
        }
        // 項9: TT手は戻す
        if m == tt.mv {
            r -= 2239;
        }

        // その手の履歴の強さを控える（yaneuraou-search.cpp:3924-3932）。
        // 子のhistory更新量にも効くのでdo_moveの前に測る
        let stat_score = self.stat_score(m, cont);
        self.stack[ply + STACK_OFFSET].stat_score = stat_score;
        // 項10: 履歴の良い手は戻し、悪い手は削る
        r -= stat_score * 428 / 4096;
        // 項11: allNodeでは全体を割り増す
        if all_node {
            r += r * 273 / (256 * depth as i32 + 260);
        }
        r
    }

    /// Step 14。浅い深さでの枝刈り（yaneuraou-search.cpp:3586-3698）。
    /// 真を返したら、その手は読まずに次の手へ進む。
    ///
    /// 前提条件は「rootでない」「bestValueが敗勢でない」の2つだけで、
    /// search()は常にrootでない。`best` は1手目を読み終えるまで
    /// -VALUE_INFINITEなので、第1手はここで刈られない。親futilityは
    /// 刈った手の見込み値で `best` を引き上げる
    // ノードで決まる値はNodeInfoへまとめてある。残りは手ごとに変わる値と、
    // 呼び出し側が書き換えるMovePicker・bestである
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prune_shallow<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        picker: &mut MovePicker,
        best: &mut Value,
        m: Move,
        alpha: Value,
        depth: u32,
        count: u32,
        r: i32,
        improving: bool,
        is_capture: bool,
        gives_check: bool,
        best_move: Move,
        cont: &[usize; 6],
    ) -> bool {
        if *best <= VALUE_MATED_IN_MAX_PLY {
            return false;
        }
        let &NodeInfo {
            in_check,
            follow_pv,
            static_eval,
            ..
        } = node;
        // move count pruning（yaneuraou-search.cpp:3592-3593）: 手数を
        // 使い切ったら、MovePickerに静かな手の生成そのものをやめさせる
        if count >= lmp_limit(depth, improving) {
            picker.skip_quiet_moves();
        }

        // lmr_depth: LMRで削ったあとに実際に読む深さ
        // （yaneuraou-search.cpp:3599-3600）。生のdepthで枝刈りを
        // 判断すると、深いノードほど閾値が大きくなり刈りすぎる。
        // 参照実装は延長を加える前の `depth - 1` を基準にする
        let mut lmr_depth = depth as i32 - 1 - r / 1024;

        if is_capture || gives_check {
            // 取る駒（駒打ちと王手だけの手ではEMPTY）と、その
            // capture history（yaneuraou-search.cpp:3612-3614）
            let captured = self.pos.piece_on(m.to()).piece_type();
            let capt_hist = self.hist.capture.get(m.piece_after(), m.to(), captured);

            // 取る手のfutility（yaneuraou-search.cpp:3616-3623）。
            // 取った駒の価値を足してもalphaに届かない手を捨てる。
            // 王手する手は対象外
            if !gives_check && lmr_depth < CAPT_FUTILITY_MAX_DEPTH {
                let futility_value = static_eval
                    + CAPT_FUTILITY_BASE()
                    + CAPT_FUTILITY_DEPTH() * lmr_depth
                    + himawari_core::piece_value(captured)
                    + CAPT_FUTILITY_HIST * capt_hist / 1024;
                if futility_value <= alpha {
                    return true;
                }
            }

            // 取る手・王手する手のSEE枝刈り
            // （yaneuraou-search.cpp:3634-3641）。許す損の額が
            // capture historyで動く。alphaが負のときは刈らない
            let margin =
                (SEE_CAPTURE_COEF() * depth as i32 + capt_hist * SEE_CAPT_HIST() / 1024).max(0);
            if alpha >= VALUE_DRAW && !self.pos.see_ge(m, -margin) {
                return true;
            }
        } else if !follow_pv || !PV {
            // 前回の反復深化のPV上にいるPVノードでは、静かな手の
            // 枝刈りを一切かけない（yaneuraou-search.cpp:3644）。
            // 前回のPVを浅い枝刈りで壊さないための仕掛けである
            //
            // 静かな手の履歴（yaneuraou-search.cpp:3646-3648）。
            // 1手前・2手前のcontinuation historyとpawn historyの和
            let to = m.to();
            let pc = m.piece_after();
            let pawn_slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
            let mut history = self.hist.cont.get(cont[0], pc, to)
                + self.hist.cont.get(cont[1], pc, to)
                + self.shared.hists.pawn.get(pawn_slot, pc, to);

            // continuation historyによる枝刈り
            // （yaneuraou-search.cpp:3650-3651）。履歴が極端に
            // 悪い手は読まない
            if history < -CONT_HIST_PRUNE_COEF * depth as i32 {
                return true;
            }

            // main historyを足してlmr_depthを補正する
            // （yaneuraou-search.cpp:3656-3661）。以降の枝刈りが
            // 使う尺度そのものが履歴で動く
            history += 71 * self.hist.main.get(self.pos.side_to_move(), m) / 32;
            lmr_depth += history / LMR_DEPTH_HIST_DIVISOR();

            // 親ノードのfutility（yaneuraou-search.cpp:3665-3682）。
            // 子を展開する前に、alphaへ届かないと見込める静かな手を
            // 捨てる。尺度は補正後のlmr_depthで、まだ最善手が
            // 見つかっていないときと静的評価がalphaを超えている
            // ときにマージンを積む
            let futility_value = static_eval
                + FUTILITY_BASE()
                + FUTILITY_NO_BEST() * Value::from(best_move == Move::NONE)
                + FUTILITY_MARGIN() * lmr_depth
                + FUTILITY_OVER_ALPHA() * Value::from(static_eval > alpha);
            if !in_check && lmr_depth < FUTILITY_MAX_DEPTH && futility_value <= alpha {
                // 刈った手の見込み値でbestValueを引き上げる。
                // 詰み圏の値は動かさない
                if *best <= futility_value
                    && best.abs() < VALUE_MATE_IN_MAX_PLY
                    && futility_value < VALUE_MATE_IN_MAX_PLY
                {
                    *best = futility_value;
                }
                return true;
            }

            // 負のSEEを持つ手の枝刈り（yaneuraou-search.cpp:3691-3698）。
            // 参照実装はここで0止めする
            let lmr_depth = lmr_depth.max(0);
            if !self
                .pos
                .see_ge(m, -SEE_QUIET_COEF() * lmr_depth * lmr_depth)
            {
                return true;
            }
        }
        false
    }

    /// singular extension（yaneuraou-search.cpp:3745-3850）。
    /// TT手を除外した検証探索がsingular_betaを下回れば、TT手だけが傑出して
    /// いると見て延長する。参照実装はムーブループの中でTT手に当たったときに
    /// 判定するが、対象はTT手だけで、TT手はMovePickerが最初に返す。ループの
    /// 手前で1回求めても同じである（ムーブループの枝刈りは第1手には効かない）。
    ///
    /// 検証値の位置で3通りに分かれる。singular_beta未満なら延長して `depth`
    /// も1手増やし、beta以上ならmulti-cutでこのノードごと刈り、間なら
    /// negative extensionになる。multi-cutと打ち切りは `Err` で返し、
    /// 呼び出し側がそのままreturnする。
    pub(super) fn singular_extension<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        beta: Value,
        depth: &mut u32,
        prev: Move,
    ) -> Result<Option<i32>, Value> {
        let &NodeInfo {
            ply,
            cut_node,
            excluded,
            static_eval,
            tt,
            ..
        } = node;
        let mut singular_ext: Option<i32> = None;
        if excluded == Move::NONE
            && ply > 0
            && tt.mv != Move::NONE
            // ttPvノードでは1手深いところから判定する
            && *depth >= SINGULAR_MIN_DEPTH + u32::from(self.stack[ply + STACK_OFFSET].tt_pv)
            && tt.bound != Bound::Upper
            && tt.bound != Bound::None
            && tt.depth >= (*depth).saturating_sub(3) as i32
            && tt.value.abs() < VALUE_MATE_IN_MAX_PLY
            // 往復手は検証探索にかけない（yaneuraou-search.cpp:3749）
            && !self.is_shuffling(tt.mv, ply)
            && self.pos.is_legal(tt.mv)
        {
            let singular_beta = tt.value
                - (SINGULAR_MARGIN
                    + SINGULAR_MARGIN_TTPV
                        * Value::from(self.stack[ply + STACK_OFFSET].tt_pv && !PV))
                    * *depth as Value
                    / SINGULAR_MARGIN_DIV;
            // 検証探索の深さは延長前のnewDepth（= *depth - 1）の半分
            // （yaneuraou-search.cpp:3758）
            let singular_depth = (*depth - 1) / 2;
            self.stack[ply + STACK_OFFSET].excluded_move = tt.mv;
            let mut verify_pv = Vec::new();
            let v = self.search::<false>(
                singular_beta - 1,
                singular_beta,
                singular_depth,
                ply,
                prev,
                &mut verify_pv,
                // 検証探索はcut_nodeを引き継ぐ
                cut_node,
            );
            self.stack[ply + STACK_OFFSET].excluded_move = Move::NONE;
            // 検証探索の再帰でstatic_evalが同値で上書きされる。念のため戻す
            self.stack[ply + STACK_OFFSET].static_eval = static_eval;
            if self.stopped() {
                return Err(VALUE_ZERO);
            }
            // 検証値の位置で3通りに分かれる。singular_beta未満なら多段延長、
            // beta以上ならmulti-cut、間ならnegative extensionになる
            if v < singular_beta {
                // 多段延長（yaneuraou-search.cpp:3777-3788）。検証値が
                // singular_betaをマージン分下回るごとに1手積み、最大+3にする。
                // 組合せ爆発を抑えるため、PVノードではマージンを大きく取って
                // 積みにくくする。TT手が取る手でない・correction値が大きい・
                // ttMoveHistoryが良い・rootから遠い、では積みやすくなる
                // 多段延長は入れない。参照実装の
                // doubleMargin / tripleMarginをそのまま移すと、本エンジンでは
                // singular率が43.5%（参照実装の設計点は1割）あるため+3まで
                // 積む機会が4倍以上になる。群全体では562局で-99.0だった。
                // 到達深さが30秒で4段落ちたのが主因である。
                // 検証窓の係数は本エンジンで成立していないので、多段化は
                // singular率を設計点へ寄せてから再訪する
                singular_ext = Some(1);
                // このノード自体の深さも1手増やす（yaneuraou-search.cpp:3788）。
                // TT手のnewDepthは増やす前の値で決まっているので、増分は
                // 残りの手とTT storeに効く
                *depth += 1;
            } else if v >= beta && v.abs() < VALUE_MATE_IN_MAX_PLY {
                // multi-cut（yaneuraou-search.cpp:3817-3821）。TT手を除いた
                // 浅い探索でもβを超えたので、このノードは「1手だけ傑出」では
                // なく複数の手がfail highすると見て、部分木をまとめて刈る。
                // 返す値はsoftbound（真の値がこれ以上と分かっている値）である
                self.hist
                    .tt_move
                    .update((-424 - 107 * *depth as i32).max(-3375));
                return Err(v);
            } else if tt.value >= beta {
                // negative extension（yaneuraou-search.cpp:3841-3850）。
                // 検証値がsingular_betaとβの間なので、singularともmulti-cutとも
                // 言えない。TT手が今のβを超えてfail highすると見込めるなら
                // 大きく削り、他の手を先に読ませる
                singular_ext = Some(-3);
            } else if cut_node {
                // 同じくcutNodeだが、TT手がβを超えるとは見込めない場合
                singular_ext = Some(-2);
            } else {
                // どの分岐にも入らない場合は延長も短縮もしない。
                // 参照実装のextensionの初期値0に対応する
                singular_ext = Some(0);
            }
        }
        Ok(singular_ext)
    }

    /// ムーブループの手前の枝刈り（
    /// yaneuraou-search.cpp:3159-3424）。razoring・RFP・NMP・IIR・ProbCutを
    /// この順に試す。刈れたらその値を返し、呼び出し側はそのままreturnする。
    ///
    /// 深さの事後補正とIIRが `depth` を、NMPの前後の再計算が `improving` を
    /// 書き換える。刈らずに抜けたときだけ呼び出し側へ反映される。
    pub(super) fn prune_before_moves<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        alpha: Value,
        beta: Value,
        depth: &mut u32,
        improving: &mut bool,
        prev: Move,
    ) -> Option<Value> {
        // 王手中はevalベースの枝刈りを一切行わない
        // （yaneuraou-search.cpp:3013-3020のgoto moves_loop）。
        // 静的評価が2手前の写しでしかないため、判断材料にできない
        if node.in_check {
            return None;
        }
        let &NodeInfo {
            ply,
            cut_node,
            all_node,
            follow_pv,
            excluded,
            key,
            raw_eval,
            static_eval,
            eval,
            corr_value,
            prior_reduction,
            tt,
            tt_capture,
            ..
        } = node;
        *improving = static_eval > self.stack[ply + STACK_OFFSET - 2].static_eval;
        // 相手の状況が悪化しているか（yaneuraou-search.cpp:3169）。
        // 普通は `static_eval == -(1手前のstatic_eval)` なので、これを
        // 上回るなら相手にとって評価が悪くなっている
        let opponent_worsening = static_eval > -self.stack[ply + STACK_OFFSET - 1].static_eval;

        // 1手前のリダクションに応じた残り深さの事後補正
        // （yaneuraou-search.cpp:3176-3179）。深く削って戻ってきた手が
        // 相手を悪くできていないなら1手足し、静的評価の和が閾値を超えて
        // いるなら1手引く
        if prior_reduction >= 3 && !opponent_worsening {
            *depth += 1;
        }
        if prior_reduction >= 2
            && *depth >= 2
            && static_eval + self.stack[ply + STACK_OFFSET - 1].static_eval > 173
        {
            *depth -= 1;
        }

        // razoring（yaneuraou-search.cpp:3191-3192）。
        // 評価がalphaを大きく下回るなら通常探索をやめ、qsearchの値を返す。
        // PVノードでないことが唯一の前提で、深さの上限はない
        if !PV && eval < alpha - RAZOR_BASE() - RAZOR_DEPTH_COEF() * (*depth * *depth) as Value {
            // razoringは非PVノード限定なので常にNonPV（yaneuraou-search.cpp:3192）
            return Some(self.qsearch::<false>(alpha, beta, ply));
        }

        // 子ノードのfutility（RFP。yaneuraou-search.cpp:3217-3227）。
        // 残り深さで評価が動きうる幅を見積り、それを引いてもβを超えるなら
        // 刈る。TTにヒットしていないノードは見積りを狭める
        let futility_mult =
            RFP_MULT() - RFP_NO_TT_HIT() * i32::from(!self.stack[ply + STACK_OFFSET].tt_hit);
        let futility_margin = futility_mult * *depth as i32
            - (RFP_IMPROVING() * i32::from(*improving)
                + RFP_OPP_WORSENING() * i32::from(opponent_worsening))
                * futility_mult
                / 1024
            + corr_value.abs() / RFP_CORR_DIVISOR;
        if !self.stack[ply + STACK_OFFSET].tt_pv
            && *depth < RFP_MAX_DEPTH
            && eval >= beta
            && eval - futility_margin >= beta
            && (tt.mv == Move::NONE || tt_capture)
            && beta > VALUE_MATED_IN_MAX_PLY
            && eval < VALUE_MATE_IN_MAX_PLY
        {
            // 静的評価そのものではなく、βへ寄せた値を返す
            return Some((2 * beta + eval) / 3);
        }

        // NMP（yaneuraou-search.cpp:3236-3301）。
        // 手番を渡して浅く探索し、それでもβ以上なら刈る。cutNode限定で、
        // 深さの下限はない。評価の閾値はβから残り深さとimprovingで割り引く。
        // 除外手つき探索中はスキップ
        if cut_node
            && static_eval
                >= beta
                    - NMP_EVAL_DEPTH * *depth as Value
                    - NMP_EVAL_IMPROVING() * Value::from(*improving)
                    + NMP_EVAL_BASE()
            && excluded == Move::NONE
            && ply >= self.nmp_min_ply
            && beta > VALUE_MATED_IN_MAX_PLY
        {
            // 連続してnull moveは指さない（yaneuraou-search.cpp:3247）。
            // null moveの子はcut_node = falseなのでここへ来ない
            debug_assert!(prev != Move::NULL);
            let r = NMP_BASE_REDUCTION() + *depth / NMP_DEPTH_DIVISOR();
            let mut null_pv = Vec::new();
            // null moveは王手でも駒取りでもないので番兵の面を指す
            // （yaneuraou-search.cpp:3254-3256）
            let e = &mut self.stack[ply + STACK_OFFSET];
            e.current_move = Move::NULL;
            e.cont_base = ContinuationHistory::SENTINEL;
            e.cont_corr_base = 0;
            self.make_null_move();
            let v = -self.search::<false>(
                -beta,
                -beta + 1,
                (*depth).saturating_sub(r),
                ply + 1,
                Move::NULL,
                &mut null_pv,
                // NMPの子はcut_node = false
                false,
            );
            self.unmake_null_move();
            if self.stopped() {
                return Some(VALUE_ZERO);
            }
            // パス由来の詰みスコアは信用しない。刈らずに読み進める
            if v >= beta && v < VALUE_MATE_IN_MAX_PLY {
                // 深いところでは同じ深さの検証探索で裏を取る
                // （yaneuraou-search.cpp:3277-3301）。zugzwangでの誤りを
                // 減らす。検証探索の中ではnmpMinPlyまでNMPを止める
                if self.nmp_min_ply != 0 || *depth < NMP_VERIFY_MIN_DEPTH {
                    return Some(v);
                }
                self.nmp_min_ply = ply + 3 * (*depth - r) as usize / 4;
                let mut verify_pv = Vec::new();
                let vv = self.search::<false>(
                    beta - 1,
                    beta,
                    *depth - r,
                    ply,
                    prev,
                    &mut verify_pv,
                    false,
                );
                self.nmp_min_ply = 0;
                if self.stopped() {
                    return Some(VALUE_ZERO);
                }
                if vv >= beta {
                    return Some(v);
                }
            }
        }

        // NMPの後にβで再計算する（yaneuraou-search.cpp:3306）。
        // 静的評価がβ以上なら、2手前と比べていなくても改善扱いにする
        *improving |= static_eval >= beta;

        // IIR（yaneuraou-search.cpp:3319-3320）。
        // TTに手がないノードは良い順序を作れないので1浅く読み、再訪時に
        // TT手付きで読み直す。前回PVの上と、全手を読むallNodeでは行わない。
        // 1手前を深く削って来たノードも対象から外す
        if !follow_pv
            && !all_node
            && *depth >= IIR_MIN_DEPTH
            && tt.mv == Move::NONE
            && prior_reduction <= IIR_MAX_PRIOR_REDUCTION
        {
            *depth -= 1;
        }

        // ProbCut（yaneuraou-search.cpp:3357-3424）。
        // betaを大きく超えそうなノードでは、浅い確認探索で「十分良い取る手が
        // 1つある」ことを示せれば高深度の全探索を省いてカットする。
        // 閾値はimprovingで動き、MovePickerがSEEでこの閾値を満たす取る手
        // だけをcapture history込みの順序で返す
        let probcut_beta = beta + PROBCUT_MARGIN - PROBCUT_IMPROVING * Value::from(*improving);
        if *depth >= PROBCUT_MIN_DEPTH
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            // 置換表の値がprobcut_beta未満と分かっているなら試さない
            && !(tt.value != VALUE_NONE && tt.value < probcut_beta)
        {
            let probcut_depth = *depth as i32 - PROBCUT_DEPTH_REDUCTION;
            // ply別の再利用バッファを借りる。このplyの
            // MovePickerは同時に1つしか生きないので、同じスロットを使える
            let buf = std::mem::take(&mut self.node_bufs[ply].moves);
            let mut picker =
                MovePicker::new_probcut(&self.pos, tt.mv, probcut_beta - static_eval, buf);
            let cont = self.cont_bases(ply);
            // バッファの返却を一本化するため、復帰点をこのブロックにまとめる
            let probcut_value = 'probcut: {
                while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
                    // 除外手はsingular検証探索中のTT手
                    if m == excluded || !self.pos.is_legal_picked(m, picker.last_from_tt()) {
                        continue;
                    }
                    self.set_current_move(ply, m, !self.pos.piece_on(m.to()).is_empty());
                    self.make_move(m);
                    // まずqsearchで確認（窓は (-probcut_beta, -probcut_beta+1)）
                    let mut v = -self.qsearch::<false>(-probcut_beta, -probcut_beta + 1, ply + 1);
                    // 通ったら同じ窓で通常探索 *depth-4 を確認する
                    if v >= probcut_beta && probcut_depth > 0 {
                        let mut child_pv = Vec::new();
                        v = -self.search::<false>(
                            -probcut_beta,
                            -probcut_beta + 1,
                            probcut_depth as u32,
                            ply + 1,
                            m,
                            &mut child_pv,
                            // ProbCutの子はcut_nodeを反転する
                            !cut_node,
                        );
                    }
                    self.unmake_move(m);
                    if self.stopped() {
                        break 'probcut Some(VALUE_ZERO);
                    }
                    if v >= probcut_beta {
                        // fail-soft。TTにlower bound・*depth-3で保存する
                        self.shared.tt.store(
                            key,
                            m.to_move16(),
                            value_to_tt(v, ply),
                            raw_eval as i16,
                            (probcut_depth + 1 + TT_DEPTH_OFFSET).clamp(0, 255) as u8,
                            Bound::Lower,
                            // 参照実装はttPvを書き戻す（yaneuraou-search.cpp:3418）
                            self.stack[ply + STACK_OFFSET].tt_pv,
                        );
                        // 決着スコアでなければ、上乗せしたマージンを戻して
                        // カットする。決着スコアのときは次の手を試す
                        if v.abs() < VALUE_MATE_IN_MAX_PLY {
                            break 'probcut Some(v - (probcut_beta - beta));
                        }
                    }
                }
                None
            };
            // 借りたバッファを戻す
            self.node_bufs[ply].moves = picker.into_buf();
            if probcut_value.is_some() {
                return probcut_value;
            }
        }
        None
    }
}
