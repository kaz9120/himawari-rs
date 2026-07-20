"""PSV dataset reader via Rust PyO3 bridge (ADR-0043, ADR-0044)."""

import numpy as np
import torch
from torch.utils.data import Dataset

import himawari


class PsvDataset(Dataset):
    """Memory-mapped PSV dataset with Rust feature extraction."""

    def __init__(self, path, lambda_=0.7, score_limit=0, arch="halfkp_effect"):
        raw = np.fromfile(path, dtype=np.uint8)
        if raw.size % 40 != 0:
            raise ValueError(f"ファイルサイズが40の倍数でない: {raw.size}")
        self.data = raw.reshape(-1, 40)
        self.lambda_ = lambda_
        self.score_limit = score_limit
        self.arch = arch

    def __len__(self):
        return len(self.data)

    def __getitem__(self, idx):
        record = bytes(self.data[idx])
        result = himawari.extract_features_v2(
            record, self.lambda_, self.score_limit, self.arch,
        )
        if result is None:
            return None
        stm_feats, opp_feats, ef_feats, target = result
        return stm_feats, opp_feats, ef_feats, target


def collate_psv(batch):
    """Collate variable-length feature lists into EmbeddingBag format."""
    batch = [b for b in batch if b is not None]
    if not batch:
        return None

    stm_all, stm_off = [], [0]
    opp_all, opp_off = [], [0]
    ef_all, ef_off = [], [0]
    targets = []

    for stm, opp, ef, t in batch:
        stm_all.extend(stm)
        stm_off.append(len(stm_all))
        opp_all.extend(opp)
        opp_off.append(len(opp_all))
        ef_all.extend(ef)
        ef_off.append(len(ef_all))
        targets.append(t)

    return (
        torch.tensor(stm_all, dtype=torch.long),
        torch.tensor(stm_off[:-1], dtype=torch.long),
        torch.tensor(opp_all, dtype=torch.long),
        torch.tensor(opp_off[:-1], dtype=torch.long),
        torch.tensor(ef_all, dtype=torch.long),
        torch.tensor(ef_off[:-1], dtype=torch.long),
        torch.tensor(targets, dtype=torch.float32),
    )
