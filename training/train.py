"""NNUE trainer v2 (ADR-0040, ADR-0045)."""

import argparse
import csv
import math
import os
import sys
import time

import torch
from torch.utils.data import DataLoader
from torch.utils.tensorboard import SummaryWriter

from model import NnueModel, loss_fn
from dataset import PsvDataset, collate_psv
from quantize import save_hmwr


def lr_lambda(step, warmup_steps, total_steps, min_lr, peak_lr):
    if step < warmup_steps:
        return step / max(warmup_steps, 1)
    progress = (step - warmup_steps) / max(total_steps - warmup_steps, 1)
    ratio = min_lr / peak_lr
    return ratio + (1.0 - ratio) * 0.5 * (1.0 + math.cos(math.pi * progress))


def validate(model, valid_loader, device):
    model.eval()
    total_loss = 0.0
    total_n = 0
    with torch.no_grad():
        for batch in valid_loader:
            if batch is None:
                continue
            stm_i, stm_o, opp_i, opp_o, targets = [
                x.to(device) for x in batch
            ]
            out = model(stm_i, stm_o, opp_i, opp_o)
            loss = loss_fn(out, targets)
            total_loss += loss.item() * targets.size(0)
            total_n += targets.size(0)
    model.train()
    return total_loss / max(total_n, 1)


def main():
    p = argparse.ArgumentParser(description="NNUE trainer v2")
    p.add_argument("--data", required=True, help="Training PSV file")
    p.add_argument("--valid", help="Validation PSV file")
    p.add_argument("--out", required=True, help="Output .hmwr path")
    p.add_argument("--epochs", type=int, default=1)
    p.add_argument("--batch", type=int, default=16384)
    p.add_argument("--peak-lr", type=float, default=1e-3)
    p.add_argument("--min-lr", type=float, default=1e-6)
    p.add_argument("--warmup-steps", type=int, default=100)
    p.add_argument("--lambda", type=float, default=0.7, dest="lambda_")
    p.add_argument("--score-limit", type=int, default=0)
    p.add_argument("--workers", type=int, default=4)
    p.add_argument("--log-interval", type=int, default=100)
    p.add_argument("--valid-interval", type=int, default=2000)
    p.add_argument("--patience", type=int, default=0, help="Early stopping patience (0=disabled)")
    p.add_argument("--log-file", help="TSV log output path")
    p.add_argument("--log-dir", help="TensorBoard log directory")
    p.add_argument("--checkpoint-dir", help="Checkpoint directory")
    p.add_argument("--resume", help="Resume from checkpoint")
    p.add_argument("--registry", help="Experiment registry TSV path")
    p.add_argument("--name", default="", help="Experiment name for registry")
    p.add_argument("--notes", default="", help="Notes for registry")
    p.add_argument("--device", default="cpu")
    args = p.parse_args()

    device = torch.device(args.device)
    print(f"Device: {device}", file=sys.stderr)

    train_ds = PsvDataset(args.data, lambda_=args.lambda_, score_limit=args.score_limit)
    train_loader = DataLoader(
        train_ds, batch_size=args.batch, shuffle=True,
        num_workers=args.workers, collate_fn=collate_psv,
        pin_memory=(device.type != "cpu"),
    )
    data_n = len(train_ds)
    steps_per_epoch = math.ceil(data_n / args.batch)
    total_steps = args.epochs * steps_per_epoch

    valid_loader = None
    if args.valid:
        valid_ds = PsvDataset(args.valid, lambda_=args.lambda_, score_limit=args.score_limit)
        valid_loader = DataLoader(
            valid_ds, batch_size=args.batch, shuffle=False,
            num_workers=args.workers, collate_fn=collate_psv,
        )
        print(f"検証データ: {len(valid_ds)}局面", file=sys.stderr)

    model = NnueModel().to(device)

    dense_params = [
        model.ft_bias,
        model.l2.weight, model.l2.bias,
        model.l3.weight, model.l3.bias,
        model.l4.weight, model.l4.bias,
    ]
    sparse_params = [model.ft.weight]
    lr_fn = lambda step: lr_lambda(
        step, args.warmup_steps, total_steps, args.min_lr, args.peak_lr,
    )
    optimizer_dense = torch.optim.Adam(dense_params, lr=args.peak_lr)
    optimizer_sparse = torch.optim.SparseAdam(sparse_params, lr=args.peak_lr)
    scheduler_dense = torch.optim.lr_scheduler.LambdaLR(optimizer_dense, lr_lambda=lr_fn)
    scheduler_sparse = torch.optim.lr_scheduler.LambdaLR(optimizer_sparse, lr_lambda=lr_fn)

    step = 0
    start_epoch = 0
    best_valid = float("inf")
    best_step = 0
    no_improve = 0

    if args.resume:
        print(f"チェックポイント復元: {args.resume}", file=sys.stderr)
        ckpt = torch.load(args.resume, map_location=device, weights_only=False)
        model.load_state_dict(ckpt["model"])
        optimizer_dense.load_state_dict(ckpt["optimizer_dense"])
        optimizer_sparse.load_state_dict(ckpt["optimizer_sparse"])
        scheduler_dense.load_state_dict(ckpt["scheduler_dense"])
        scheduler_sparse.load_state_dict(ckpt["scheduler_sparse"])
        step = ckpt["step"]
        start_epoch = ckpt["epoch"]
        best_valid = ckpt.get("best_valid", float("inf"))
        best_step = ckpt.get("best_step", 0)

    print(
        f"学習データ: {data_n}局面 × {args.epochs}エポック, "
        f"batch={args.batch}, peak_lr={args.peak_lr}, "
        f"warmup={args.warmup_steps}, total_steps={total_steps}, "
        f"λ={args.lambda_}",
        file=sys.stderr,
    )

    writer = SummaryWriter(args.log_dir) if args.log_dir else None
    log_file = None
    if args.log_file:
        is_new = not os.path.exists(args.log_file)
        log_file = open(args.log_file, "a")
        if is_new:
            log_file.write("type\tstep\tepoch\tsamples\tloss\tlr\tsps\telapsed_s\n")

    if args.checkpoint_dir:
        os.makedirs(args.checkpoint_dir, exist_ok=True)

    t0 = time.time()
    t_log = time.time()
    loss_acc = 0.0
    loss_n = 0
    samples_log = 0
    samples_done = step * args.batch
    early_stopped = False

    model.train()
    for epoch in range(start_epoch, args.epochs):
        for batch in train_loader:
            if batch is None:
                continue

            stm_i, stm_o, opp_i, opp_o, targets = [
                x.to(device) for x in batch
            ]
            n = targets.size(0)

            optimizer_dense.zero_grad()
            optimizer_sparse.zero_grad()
            out = model(stm_i, stm_o, opp_i, opp_o)
            loss = loss_fn(out, targets)
            loss.backward()
            optimizer_dense.step()
            optimizer_sparse.step()
            scheduler_dense.step()
            scheduler_sparse.step()
            model.clip_weights()

            step += 1
            samples_done += n
            samples_log += n
            loss_acc += loss.item() * n
            loss_n += n

            if step % args.log_interval == 0:
                elapsed = time.time() - t_log
                sps = samples_log / max(elapsed, 1e-6)
                avg_loss = loss_acc / max(loss_n, 1)
                current_lr = scheduler_dense.get_last_lr()[0]
                print(
                    f"step {step} samples {samples_done} "
                    f"loss {avg_loss:.5f} lr {current_lr:.6f} "
                    f"({sps:.0f} samples/s)",
                    file=sys.stderr,
                )
                if writer:
                    writer.add_scalar("train/loss", avg_loss, step)
                    writer.add_scalar("train/lr", current_lr, step)
                    writer.add_scalar("train/sps", sps, step)
                if log_file:
                    total_elapsed = time.time() - t0
                    log_file.write(
                        f"train\t{step}\t{epoch}\t{samples_done}\t"
                        f"{avg_loss:.5f}\t{current_lr:.6f}\t"
                        f"{sps:.0f}\t{total_elapsed:.1f}\n"
                    )
                    log_file.flush()
                loss_acc = 0.0
                loss_n = 0
                t_log = time.time()
                samples_log = 0

            if valid_loader and step % args.valid_interval == 0:
                vl = validate(model, valid_loader, device)
                print(f"  valid loss {vl:.5f}", file=sys.stderr)
                if writer:
                    writer.add_scalar("valid/loss", vl, step)
                if log_file:
                    total_elapsed = time.time() - t0
                    log_file.write(
                        f"valid\t{step}\t{epoch}\t{samples_done}\t"
                        f"{vl:.5f}\t\t\t{total_elapsed:.1f}\n"
                    )
                    log_file.flush()

                if vl < best_valid:
                    best_valid = vl
                    best_step = step
                    no_improve = 0
                    best_path = f"{args.out}.best"
                    lineage = (
                        f"train-v2-pytorch data={args.data} n={data_n} "
                        f"step={step} valid_loss={vl:.5f} "
                        f"batch={args.batch} peak_lr={args.peak_lr} "
                        f"lambda={args.lambda_}"
                    )
                    save_hmwr(model, lineage, best_path)
                    print(
                        f"  best checkpoint: {best_path} "
                        f"(step {step}, valid {vl:.5f})",
                        file=sys.stderr,
                    )
                    if args.checkpoint_dir:
                        _save_checkpoint(
                            model, optimizer_dense, optimizer_sparse,
                            scheduler_dense, scheduler_sparse,
                            step, epoch, best_valid, best_step, samples_done,
                            os.path.join(args.checkpoint_dir, "best.ckpt"),
                        )
                else:
                    no_improve += 1

                if args.checkpoint_dir:
                    _save_checkpoint(
                        model, optimizer_dense, optimizer_sparse,
                        scheduler_dense, scheduler_sparse,
                        step, epoch, best_valid, best_step, samples_done,
                        os.path.join(args.checkpoint_dir, "latest.ckpt"),
                    )

                if args.patience > 0 and no_improve >= args.patience:
                    print(
                        f"Early stopping: {no_improve}回改善なし "
                        f"(patience={args.patience})",
                        file=sys.stderr,
                    )
                    early_stopped = True
                    break

        if early_stopped:
            break
        print(f"epoch {epoch + 1} 完了", file=sys.stderr)

    elapsed_total = time.time() - t0
    if valid_loader:
        final_valid = validate(model, valid_loader, device)
        print(
            f"最終valid loss {final_valid:.5f} "
            f"(best {best_valid:.5f} at step {best_step})",
            file=sys.stderr,
        )
    else:
        final_valid = float("nan")

    print(
        f"学習完了: {step}ステップ {samples_done}局面 {elapsed_total:.1f}秒",
        file=sys.stderr,
    )

    lineage = (
        f"train-v2-pytorch data={args.data} n={data_n} "
        f"epochs={args.epochs} batch={args.batch} "
        f"peak_lr={args.peak_lr} min_lr={args.min_lr} "
        f"warmup={args.warmup_steps} lambda={args.lambda_} "
        f"steps={step}"
    )
    save_hmwr(model, lineage, args.out)
    print(f"{args.out} を書き出しました", file=sys.stderr)

    if args.registry:
        _append_registry(args, data_n, step, total_steps,
                         best_step, best_valid, final_valid, elapsed_total)

    if writer:
        writer.close()
    if log_file:
        log_file.close()


def _save_checkpoint(model, optimizer_dense, optimizer_sparse,
                     scheduler_dense, scheduler_sparse,
                     step, epoch, best_valid, best_step, samples, path):
    torch.save({
        "model": model.state_dict(),
        "optimizer_dense": optimizer_dense.state_dict(),
        "optimizer_sparse": optimizer_sparse.state_dict(),
        "scheduler_dense": scheduler_dense.state_dict(),
        "scheduler_sparse": scheduler_sparse.state_dict(),
        "step": step,
        "epoch": epoch,
        "best_valid": best_valid,
        "best_step": best_step,
        "samples": samples,
    }, path)


def _append_registry(args, data_n, step, total_steps,
                     best_step, best_valid, final_valid, elapsed):
    is_new = not os.path.exists(args.registry)
    with open(args.registry, "a", newline="") as f:
        w = csv.writer(f, delimiter="\t")
        if is_new:
            w.writerow([
                "timestamp", "name", "data", "data_n", "epochs", "batch",
                "peak_lr", "min_lr", "warmup", "lambda", "best_step",
                "best_valid", "final_valid", "total_steps", "elapsed_s", "notes",
            ])
        import datetime
        w.writerow([
            datetime.datetime.now().isoformat(timespec="seconds"),
            args.name, args.data, data_n, args.epochs, args.batch,
            args.peak_lr, args.min_lr, args.warmup_steps, args.lambda_,
            best_step, f"{best_valid:.5f}", f"{final_valid:.5f}",
            total_steps, f"{elapsed:.0f}", args.notes,
        ])


if __name__ == "__main__":
    main()
