// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed parameters for operator-specific healing and tactical callbacks.
use super::*;
use crate::battle::runtime::kits;

#[derive(Default)]
pub(crate) struct TacticalState {
    pub next: [f64; 5],
    state: OperatorState,
}
// Each declaration binds the parameters, behavior and optional mutable state.
// State reset/redeployment policies remain explicit in TacticalState::reset.
// Brackets contain the optional immutable state accessor.
macro_rules! operator_tactics {
    ($( $variant:ident($module:ident::$parameters:ident $(, $state:ident, [$($get:ident)?], $get_mut:ident, $error:literal)?); )*) => {
        #[derive(Default)]
        pub(crate) enum Tactics {
            #[default]
            None,
            $( $variant(kits::$module::$parameters), )*
        }

        impl Tactics {
            pub(crate) fn behavior(&self) -> Option<&dyn kits::dispatch::Behavior> {
                match self {
                    Self::None => None,
                    $( Self::$variant(kit) => Some(kit), )*
                }
            }
        }

        #[derive(Default)]
        enum OperatorState {
            #[default]
            None,
            $( $( $variant(kits::$module::$state), )? )*
        }

        impl TacticalState {
            $( $(
                $(
                    pub fn $get(&self) -> Result<&kits::$module::$state, String> {
                        match &self.state {
                            OperatorState::$variant(state) => Ok(state),
                            _ => Err($error.into()),
                        }
                    }
                )?
                pub fn $get_mut(&mut self) -> Result<&mut kits::$module::$state, String> {
                    match &mut self.state {
                        OperatorState::$variant(state) => Ok(state),
                        _ => Err($error.into()),
                    }
                }
            )? )*
        }
    };
}
operator_tactics! {
    Clemnt(clemnt::Clemnt, State, [clemnt], clemnt_mut, "Invalid Clement state");
    PreparedSummon(prepared_summon::PreparedSummon, State, [prepared_summon], prepared_summon_mut, "Missing prepared summon state");
    Yanyou(yanyou::Yanyou, State, [yanyou], yanyou_mut, "Missing yanyou state");
    Mlyss(mlyss::Mlyss, State, [mlyss], mlyss_mut, "Invalid mlyss kit state");
    Manifold(manifold::Manifold, State, [manifold], manifold_mut, "Missing manifold state");
    Vigil(vigil::Vigil, State, [vigil], vigil_mut, "Invalid vigil kit state");
    Wolf(wolf::Wolf, State, [wolf], wolf_mut, "Missing wolf state");
    Etlchi(etlchi::Etlchi, State, [etlchi], etlchi_mut, "Invalid etlchi kit state");
    Whitw2(whitw2::Whitw2, State, [whitw2], whitw2_mut, "Invalid whitw2 kit state");
    Swire2(swire2::Swire2, State, [swire2], swire2_mut, "Invalid swire2 kit state");
    Ulpia(ulpia::Ulpia, State, [], ulpia_mut, "Invalid ulpia kit state");
    Siege2(siege2::Siege2, State, [siege2], siege2_mut, "Invalid siege2 kit state");
    Nearl2(nearl2::Nearl2, State, [nearl2], nearl2_mut, "Invalid nearl2 kit state");
    Lemuen(lemuen::Lemuen, State, [lemuen], lemuen_mut, "Invalid lemuen kit state");
    Kazema(kazema::Kazema, State, [kazema], kazema_mut, "Invalid kazema kit state");
    Dusk(dusk::Dusk, State, [dusk], dusk_mut, "Invalid dusk kit state");
    Yu(yu::Yu, State, [yu], yu_mut, "Invalid yu kit state");
    Thorn2(thorn2::Thorn2, State, [thorn2], thorn2_mut, "Invalid thorn2 kit state");
    Angel2(angel2::Angel2, State, [angel2], angel2_mut, "Invalid angel2 kit state");
    Svash2(svash2::Svash2, State, [svash2], svash2_mut, "Invalid svash2 kit state");
    Skadi2(skadi2::Skadi2, State, [skadi2], skadi2_mut, "Invalid skadi2 kit state");
    Sbell2(sbell2::Sbell2, State, [sbell2], sbell2_mut, "Invalid sbell2 kit state");
    Reed2(reed2::Reed2, State, [], reed2_mut, "Invalid reed2 kit state");
    Rosmon(rosmon::Rosmon);
    Caper(caper::Caper, State, [caper], caper_mut, "Invalid caper kit state");
    Indigo(indigo::Indigo, State, [indigo], indigo_mut, "Invalid indigo kit state");
    Silent(silent::Silent);
    Rockr(rockr::Rockr, State, [rockr], rockr_mut, "Invalid rockr kit state");
    Tippi(tippi::Tippi, State, [tippi], tippi_mut, "Invalid tippi kit state");
    Malist(malist::Malist, State, [malist], malist_mut, "Invalid malist kit state");
    Vulpis(vulpis::Vulpis, State, [vulpis], vulpis_mut, "Invalid vulpis kit state");
    Kjera(kjera::Kjera, State, [kjera], kjera_mut, "Invalid kjera kit state");
    Ines(ines::Ines, State, [ines], ines_mut, "Invalid ines kit state");
    Cetsyr(cetsyr::Cetsyr, State, [cetsyr], cetsyr_mut, "Invalid cetsyr kit state");
    Excu2(excu2::Excu2, State, [excu2], excu2_mut, "Invalid excu2 kit state");
    Titi(titi::Titi, State, [titi], titi_mut, "Invalid titi kit state");
    Blaze2(blaze2::Blaze2, State, [blaze2], blaze2_mut, "Invalid blaze2 kit state");
    Horn(horn::Horn, State, [horn], horn_mut, "Invalid horn kit state");
    Ghost2(ghost2::Ghost2, State, [ghost2], ghost2_mut, "Invalid ghost2 kit state");
    Nymph(nymph::Nymph);
    Cello(cello::Cello, State, [cello], cello_mut, "Invalid cello kit state");
    Halo2(halo2::Halo2, State, [halo2], halo2_mut, "Invalid halo2 kit state");
    Blkkgt(blkkgt::Blkkgt, State, [blkkgt], blkkgt_mut, "Invalid blkkgt kit state");
    Agoat2(agoat2::Agoat2, State, [agoat2], agoat2_mut, "Invalid agoat2 kit state");
    Papyrs(papyrs::Papyrs, State, [papyrs], papyrs_mut, "Invalid papyrs kit state");
    Tinman(tinman::Tinman, State, [tinman], tinman_mut, "Invalid tinman kit state");
    Philae(philae::Philae, State, [philae], philae_mut, "Invalid philae kit state");
    Snhunt(snhunt::Snhunt, State, [snhunt], snhunt_mut, "Invalid snhunt kit state");
    Blemsh(blemsh::Blemsh, State, [blemsh], blemsh_mut, "Invalid blemsh kit state");
    Archet(archet::Archet, State, [archet], archet_mut, "Invalid archet kit state");
    Rmixer(rmixer::Rmixer, State, [rmixer], rmixer_mut, "Invalid rmixer kit state");
    Texas2(texas2::Texas2, State, [texas2], texas2_mut, "Invalid texas2 kit state");
    Demkni(demkni::Demkni, State, [demkni], demkni_mut, "Invalid demkni kit state");
    Mlynar(mlynar::Mlynar, State, [mlynar], mlynar_mut, "Invalid mlynar kit state");
    Sntlla(sntlla::Sntlla, State, [sntlla], sntlla_mut, "Invalid sntlla kit state");
    Pasngr(pasngr::Pasngr);
    Qiubai(qiubai::Qiubai, State, [qiubai], qiubai_mut, "Invalid qiubai kit state");
    Pepe(pepe::Pepe, State, [pepe], pepe_mut, "Invalid pepe kit state");
    Lumen(lumen::Lumen, State, [lumen], lumen_mut, "Invalid lumen kit state");
}
pub(crate) fn n(v: &Value, key: &str, default: f64) -> f64 {
    number(&v[key], default)
}
pub(crate) fn suffix_number(v: &Value, key: &str, default: f64) -> f64 {
    v.as_object()
        .and_then(|o| {
            o.get(key).or_else(|| {
                o.iter()
                    .find(|(k, _)| k.ends_with(&format!(".{key}")))
                    .map(|(_, v)| v)
            })
        })
        .map(|v| number(v, default))
        .unwrap_or(default)
}
pub(crate) fn tactical_mode(raw: &Value) -> Result<Mode, String> {
    match raw["skill"]["skillId"]
        .as_str()
        .unwrap_or("")
        .rsplit('_')
        .next()
        .unwrap_or("")
    {
        "1" => Ok(Mode::First),
        "2" => Ok(Mode::Second),
        "3" => Ok(Mode::Third),
        // Shared quick-attack skill is the first skill of 寒檀.
        "quickattack[3]" => Ok(Mode::First),
        _ => Err("Unknown tactical operator skill".into()),
    }
}
pub(crate) fn tactical_grid(v: &Value, box_fallback: bool) -> Result<Vec<[i32; 2]>, String> {
    if v.as_array().is_some_and(|a| !a.is_empty()) {
        return crate::battle::operators::parse_range(v);
    }
    Ok(if box_fallback {
        (-1..=1)
            .flat_map(|r| (-1..=1).map(move |c| [r, c]))
            .collect()
    } else {
        vec![[0, 0]]
    })
}

impl TacticalState {
    pub(crate) fn dollkeeper(
        &self,
    ) -> Result<&crate::battle::runtime::kits::dollkeeper::State, String> {
        match &self.state {
            OperatorState::Ghost2(s) => Ok(&s.dollkeeper),
            OperatorState::Kazema(s) => Ok(&s.dollkeeper),
            _ => Err("Missing dollkeeper state".into()),
        }
    }
    pub(crate) fn dollkeeper_mut(
        &mut self,
    ) -> Result<&mut crate::battle::runtime::kits::dollkeeper::State, String> {
        match &mut self.state {
            OperatorState::Ghost2(s) => Ok(&mut s.dollkeeper),
            OperatorState::Kazema(s) => Ok(&mut s.dollkeeper),
            _ => Err("Missing dollkeeper state".into()),
        }
    }

    pub fn reset(&mut self, tactics: &Tactics) {
        use crate::battle::runtime::kits;
        let state = match tactics {
            Tactics::PreparedSummon(_) => {
                let previous = match &self.state {
                    OperatorState::PreparedSummon(previous) => Some(previous),
                    _ => None,
                };
                OperatorState::PreparedSummon(kits::prepared_summon::State::redeployed(previous))
            }
            Tactics::Mlyss(_) => OperatorState::Mlyss(
                crate::battle::runtime::kits::mlyss::State::redeployed(match &self.state {
                    OperatorState::Mlyss(previous) => Some(previous),
                    _ => None,
                }),
            ),
            Tactics::Yanyou(_) => OperatorState::Yanyou(Default::default()),
            Tactics::Manifold(_) => OperatorState::Manifold(Default::default()),
            Tactics::Vigil(_) => OperatorState::Vigil(Default::default()),
            Tactics::Wolf(_) => OperatorState::Wolf(match &self.state {
                OperatorState::Wolf(previous) => {
                    crate::battle::runtime::kits::wolf::State::redeployed(previous)
                }
                _ => Default::default(),
            }),
            Tactics::Etlchi(_) => OperatorState::Etlchi(Default::default()),
            Tactics::Whitw2(_) => OperatorState::Whitw2(Default::default()),
            Tactics::Swire2(_) => OperatorState::Swire2(
                crate::battle::runtime::kits::swire2::State::redeployed(match &self.state {
                    OperatorState::Swire2(previous) => Some(previous),
                    _ => None,
                }),
            ),
            Tactics::Ulpia(_) => OperatorState::Ulpia(Default::default()),
            Tactics::Siege2(_) => OperatorState::Siege2(
                crate::battle::runtime::kits::siege2::State::redeployed(match &self.state {
                    OperatorState::Siege2(previous) => Some(previous),
                    _ => None,
                }),
            ),
            Tactics::Nearl2(_) => OperatorState::Nearl2(Default::default()),
            Tactics::Lemuen(_) => OperatorState::Lemuen(Default::default()),
            Tactics::Kazema(_) => {
                OperatorState::Kazema(kits::kazema::State::redeployed(self.kazema().ok()))
            }
            Tactics::Dusk(_) => OperatorState::Dusk(Default::default()),
            Tactics::Yu(_) => OperatorState::Yu(kits::yu::State::redeployed(self.yu().ok())),
            Tactics::Thorn2(_) => {
                OperatorState::Thorn2(kits::thorn2::State::redeployed(self.thorn2().ok()))
            }
            Tactics::Angel2(_) => OperatorState::Angel2(Default::default()),
            Tactics::Svash2(_) => {
                OperatorState::Svash2(kits::svash2::State::redeployed(self.svash2().ok()))
            }
            Tactics::Skadi2(_) => {
                OperatorState::Skadi2(kits::skadi2::State::redeployed(self.skadi2().ok()))
            }
            Tactics::Sbell2(_) => OperatorState::Sbell2(Default::default()),
            Tactics::Reed2(_) => OperatorState::Reed2(Default::default()),
            Tactics::Caper(_) => OperatorState::Caper(Default::default()),
            Tactics::Indigo(_) => OperatorState::Indigo(Default::default()),
            Tactics::Rockr(_) => OperatorState::Rockr(Default::default()),
            Tactics::Tippi(_) => OperatorState::Tippi(Default::default()),
            Tactics::Malist(_) => OperatorState::Malist(Default::default()),
            Tactics::Vulpis(_) => {
                OperatorState::Vulpis(kits::vulpis::State::redeployed(self.vulpis().ok()))
            }
            Tactics::Kjera(_) => OperatorState::Kjera(Default::default()),
            Tactics::Ines(_) => {
                OperatorState::Ines(kits::ines::State::redeployed(self.ines().ok()))
            }
            Tactics::Cetsyr(_) => {
                OperatorState::Cetsyr(kits::cetsyr::State::redeployed(self.cetsyr().ok()))
            }
            Tactics::Excu2(_) => OperatorState::Excu2(Default::default()),
            Tactics::Titi(_) => {
                OperatorState::Titi(kits::titi::State::redeployed(self.titi().ok()))
            }
            Tactics::Blaze2(_) => OperatorState::Blaze2(Default::default()),
            Tactics::Horn(_) => {
                OperatorState::Horn(kits::horn::State::redeployed(self.horn().ok()))
            }
            Tactics::Ghost2(_) => OperatorState::Ghost2(Default::default()),
            Tactics::Cello(_) => {
                OperatorState::Cello(kits::cello::State::redeployed(self.cello().ok()))
            }
            Tactics::Halo2(_) => {
                OperatorState::Halo2(kits::halo2::State::redeployed(self.halo2().ok()))
            }
            Tactics::Blkkgt(_) => {
                OperatorState::Blkkgt(kits::blkkgt::State::redeployed(self.blkkgt().ok()))
            }
            Tactics::Agoat2(_) => {
                OperatorState::Agoat2(kits::agoat2::State::redeployed(self.agoat2().ok()))
            }

            Tactics::Blemsh(_) => OperatorState::Blemsh(Default::default()),
            Tactics::Demkni(_) => OperatorState::Demkni(Default::default()),
            Tactics::Papyrs(_) => OperatorState::Papyrs(Default::default()),
            Tactics::Tinman(_) => {
                OperatorState::Tinman(kits::tinman::State::redeployed(self.tinman().ok()))
            }
            Tactics::Philae(_) => OperatorState::Philae(Default::default()),
            Tactics::Snhunt(p) => OperatorState::Snhunt(kits::snhunt::State::deployed(p)),
            Tactics::Archet(_) => {
                OperatorState::Archet(kits::archet::State::redeployed(self.archet().ok()))
            }
            Tactics::Rmixer(_) => OperatorState::Rmixer(Default::default()),
            Tactics::Clemnt(_) => OperatorState::Clemnt(Default::default()),
            Tactics::Texas2(_) => {
                OperatorState::Texas2(kits::texas2::State::redeployed(self.texas2().ok()))
            }
            Tactics::Mlynar(p) => OperatorState::Mlynar(kits::mlynar::State::deployed(p)),
            Tactics::Sntlla(_) => OperatorState::Sntlla(Default::default()),
            Tactics::Qiubai(_) => OperatorState::Qiubai(Default::default()),
            Tactics::Pepe(_) => {
                OperatorState::Pepe(kits::pepe::State::redeployed(self.pepe().ok()))
            }
            Tactics::Lumen(_) => OperatorState::Lumen(Default::default()),
            _ => OperatorState::None,
        };
        *self = Self {
            next: self.next,
            state,
        };
    }
}
