"""Explicit learner device selection; CPU actors remain independent."""
import torch


def available(device):
    if device == 'cpu':
        return True
    if device == 'mps':
        return torch.backends.mps.is_available()
    if device == 'cuda':
        return torch.cuda.is_available()
    raise ValueError('Select cpu/mps/cuda explicitly')


def synchronize(device):
    if device == 'mps':
        torch.mps.synchronize()
    elif device == 'cuda':
        torch.cuda.synchronize()
