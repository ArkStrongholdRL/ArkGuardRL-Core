"""Explicit framework configuration, budgets and provenance; no research plan imports."""
from dataclasses import asdict, dataclass, replace
import math
from pathlib import Path
import shutil
import time
from arkguardrl import catalog, learn, neural, rewards
from arkguardrl.contracts import fingerprint
from . import checkpoint
from .device import available
from .identity import source_hashes
from .io import sha, write


@dataclass(frozen=True)
class SamplerConfig:
    max_decisions: int = 1024
    max_advances: int = 8192
    game_wall: float = 210
    max_episode_bytes: int = 512*1024**2
    full_trace_every: int = 16
    trace_max_bytes: int = 64*1024**2
    prep_action_cost: float = 0.0
    construction_potential: dict | None = None
    end_prep_loss: dict | None = None

    def __post_init__(self):
        from .construction import validate
        validate(self.construction_potential)
        from .ready_cost import validate as validate_ready_cost
        validate_ready_cost(self.end_prep_loss)
        for name in ('max_decisions', 'max_advances', 'max_episode_bytes', 'full_trace_every', 'trace_max_bytes'):
            if type(getattr(self, name)) is not int or getattr(self, name) < 1:
                raise ValueError('Positive integer sampler bound required: '+name)
        if not math.isfinite(self.game_wall) or self.game_wall <= 0:
            raise ValueError('Positive finite per-game wall limit required')
        if not math.isfinite(self.prep_action_cost) or self.prep_action_cost < 0:
            raise ValueError('Explicit finite nonnegative PREP attempt cost required')


@dataclass(frozen=True)
class PPOConfig:
    batch_size: int = 256
    epochs: int = 2
    learning_rate: float = 3e-5
    gae_lambda: float = .95
    clip: float = .1
    entropy_coef: float = .001
    value_coef: float = .5
    kl_coef: float = .1
    target_old_kl: float = .02
    max_grad_norm: float = .5
    max_buffer_bytes: int = 1024**3
    execution_memory_bytes: int = 1024**3

    def __post_init__(self):
        if type(self.batch_size) is not int or not 1 <= self.batch_size <= 256:
            raise ValueError('Batch size must be 1..256')
        if type(self.epochs) is not int or self.epochs < 1:
            raise ValueError('Positive integer epochs required')
        if type(self.max_buffer_bytes) is not int or self.max_buffer_bytes < 1:
            raise ValueError('Positive integer live-buffer bound required')
        if type(self.execution_memory_bytes) is not int or self.execution_memory_bytes < 1:
            raise ValueError('Positive integer padded-execution bound required')
        values = asdict(self)
        if not all(math.isfinite(v) for v in values.values()):
            raise ValueError('Finite PPO settings required')
        if not 0 <= self.gae_lambda <= 1 or not 0 < self.clip < 1:
            raise ValueError('PPO lambda/clip outside domain')
        if min(self.learning_rate, self.target_old_kl, self.max_grad_norm) <= 0:
            raise ValueError('Positive learning rate, KL threshold and gradient limit required')
        if min(self.entropy_coef, self.value_coef, self.kl_coef) < 0:
            raise ValueError('Nonnegative PPO coefficients required')


class Run:
    schema = 'arkguardrl-core/live-run/v1'

    def __init__(self, output, *, engine_root, expected_data_sha256, difficulty='ABYSS',
                 sampler=None, ppo=None, device='cpu', max_wall=600,
                 max_disk_bytes=4*1024**3, min_free_bytes=2*1024**3):
        self.sampler, self.ppo = sampler or SamplerConfig(), ppo or PPOConfig()
        if not available(device):
            raise ValueError('Requested learner device unavailable: '+device)
        if not math.isfinite(max_wall) or max_wall <= 0:
            raise ValueError('Positive finite run wall budget required')
        if type(max_disk_bytes) is not int or max_disk_bytes < 1 or type(min_free_bytes) is not int or min_free_bytes < 0:
            raise ValueError('Explicit disk budgets required')
        self.root = self.out = Path(output).resolve()
        self.engine_root = Path(engine_root).resolve(strict=True)
        self.device = device
        self.config, self.training = asdict(self.sampler), asdict(self.ppo)
        self.deadline = time.monotonic()+max_wall
        self.collection_index = 0
        self.public = catalog.PublicCatalog(self.engine_root, expected_data_sha256=expected_data_sha256)
        self.contract = learn.make_contract(self.public, difficulty)
        learn._validate_learning_contract(self.contract)
        self.cost_arm = self.sampler.prep_action_cost > 0
        objective = {'base': rewards.metadata(), 'prep_attempt_cost': self.sampler.prep_action_cost,
                     'charge': 'before.phase==PREP and action.t not in (g.ready,g.choice)',
                     'gamma': self.contract.gamma}
        if self.sampler.construction_potential is not None:
            cfg=self.sampler.construction_potential
            rules=self.public.bond_rules()
            if cfg['bond'] not in rules:
                raise ValueError('Construction covenant absent from public catalog')
            for identifier in cfg['anchors']:
                rec=self.public.record('chess',identifier)
                if rec.get('isGolden') or cfg['bond'] not in rec.get('bonds',[]):
                    raise ValueError('Construction anchor must be a normal public covenant member')
            if self.contract.gamma != 1.:
                raise ValueError('Construction potential implementation requires native gamma1')
            objective['construction_potential']=cfg
        if self.sampler.end_prep_loss is not None:
            cfg=self.sampler.end_prep_loss
            if not set(cfg['bonds'])<=set(self.public.bond_rules()):
                raise ValueError('End-PREP loss covenant absent from public catalog')
            objective['end_prep_loss']=cfg
        cost_schema = 'arkguardrl-core/prep-attempt-cost/'+fingerprint(objective)+'/v1'
        self.objective_contract = (replace(self.contract, reward_schema=cost_schema)
            if self.cost_arm or self.sampler.construction_potential is not None
            or self.sampler.end_prep_loss is not None else self.contract)
        self._sources, self._native_sources = source_hashes(), neural.source_hashes()
        self._last_source_check = -float('inf')
        self._configuration = fingerprint({'sampler': self.config, 'ppo': self.training})
        self._max_disk, self._min_free, self._last_scan = max_disk_bytes, min_free_bytes, -float('inf')
        self.root.mkdir(parents=True, exist_ok=False)
        write(self.root/'run.json', {'schema': self.schema, 'contract': self.contract.to_dict(),
            'objective_contract': self.objective_contract.to_dict(), 'reward': objective,
            'sampler': self.config, 'ppo': self.training, 'learner_device': device,
            'core_sources': self._sources, 'native_sources': self._native_sources,
            'max_wall': max_wall, 'max_disk_bytes': max_disk_bytes, 'min_free_bytes': min_free_bytes,
            'resume': 'not implicit; optimizer/RNG and exact source identity required'})
        self.check()

    def check(self, *, sources=False):
        if time.monotonic() >= self.deadline:
            raise TimeoutError('Framework run wall budget exhausted')
        if shutil.disk_usage(self.root).free < self._min_free:
            raise RuntimeError('Framework minimum free disk reached')
        if fingerprint({'sampler': self.config, 'ppo': self.training}) != self._configuration:
            raise ValueError('Run configuration changed during live work')
        now = time.monotonic()
        # Polling each row keeps budgets responsive. Hash the source closure at
        # most once per second there, and unconditionally at work boundaries.
        if sources or now-self._last_source_check >= 1:
            if source_hashes() != self._sources or neural.source_hashes() != self._native_sources:
                raise ValueError('Framework source identity changed during live work')
            self._last_source_check = now
        if now-self._last_scan >= 30:
            size = 0
            for path in self.root.rglob('*'):
                try:
                    if path.is_file():
                        size += path.stat().st_size
                except FileNotFoundError:
                    # A collector may replace a raw trace with its gzip archive.
                    continue
            if size > self._max_disk:
                raise RuntimeError('Framework disk byte limit reached')
            self._last_scan = now

    def bind_policy(self, model, path, revision):
        if type(revision) is not int or revision < 0:
            raise ValueError('Nonnegative policy revision required')
        checkpoint.verify_bound(model, path, self.contract)

    def save(self, path, model, **metadata):
        self.check(sources=True)
        checkpoint.save(path, model, self.contract, {
            'schema': self.schema, 'run_sha256': sha(self.root/'run.json'),
            'objective_contract': self.objective_contract.to_dict(),
            'sampler': self.config, 'ppo': self.training, 'learner_device': self.device, **metadata})
