# Attribution and resource boundary

Code is GPL-3.0-or-later; see LICENSE. Derived from HarcoChen/ArkGuardRL, whose
retained research originated from LittleDrinks/arknights at
`e00a2d7e44a004052c88d04d4346989ffb553bb3`. The native arkguardrl package was independently
maintained in that project; it does not import the legacy experiments/arknights code.

Environment contracts and native feature kernels are extracted from the owner's
working tree. Strategy, recording and PPO implementations are maintained in the
research repository. Repackaging is not a new gameplay result.

Stronghold Protocol (https://github.com/sganggs/Stronghold-Protocol) remains an external
clean checkout pinned to `c9d804f470b8d7422ae04eb66faf671cef21ad52` (app0.3.1).
Its GPL code license does not grant redistribution rights to official/derived game
characters, text, datasets, images, music, models or other assets. Consult that
project's NOTICE.md and THIRD-PARTY-NOTICES.md and prepare resources separately.
No such engine files, resource data, weights, trajectories or human demonstrations
are included in this repository. This is an unofficial simulator research framework.

The new native foundations in crates/engine port RNG, virtual scheduling and
deterministic math from Stronghold Protocol commit
41bbca5712ac08394bbc4d5b4e3411c6be1634b6 (app 0.3.3), under GPL-3.0-or-later.
The trigonometric routines retain the upstream fdlibm-based algorithms and
operation order. The development oracle in tools/reference uses that external
checkout; it is not part of the installed environment package.
