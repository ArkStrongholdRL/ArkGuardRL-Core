"""A reusable deadline guard around the unchanged native environment."""
import time
from arkguardrl.environment import SoloEnv


class RunBudgetExceeded(TimeoutError):
    pass


class DeadlineEnv(SoloEnv):
    def __init__(self, *args, deadline, **kwargs):
        self.deadline = deadline
        super().__init__(*args, **kwargs)
        self._timeout_limit = self.session.timeout

    def _remaining(self):
        left = self.deadline-time.monotonic()
        if left <= 0:
            raise RunBudgetExceeded('Native run wall budget exhausted')
        self.session.timeout = min(self._timeout_limit, left)

    def __enter__(self):
        self._remaining()
        return super().__enter__()

    def _call(self, method, **kwargs):
        self._remaining()
        return super()._call(method, **kwargs)

    def __exit__(self, exc_type, *args):
        if exc_type is not None and issubclass(exc_type, RunBudgetExceeded):
            self._reason = 'wall-budget'
            self.close(error=False)
        else:
            super().__exit__(exc_type, *args)
