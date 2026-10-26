use crate::effects::{self, EffectInstance, EffectRole, ParamValue};
use crate::timeline::{Clip, Document, ObjectKind, ProjectSettings};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    version: u32,
    settings: ProjectSettings,
    clips: Vec<SavedClip>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedClip {
    id: u64,
    kind: SavedKind,
    effects: Vec<SavedEffect>,
    layer: usize,
    start: u64,
    length: u64,
    source_start: u64,
}

#[derive(Serialize, Deserialize)]
enum SavedKind {
    Input {
        input: SavedEffect,
        output: SavedEffect,
    },
    Camera {
        settings: SavedEffect,
    },
    Filter,
    Effect,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedEffect {
    id: u64,
    type_id: String,
    enabled: bool,
    params: BTreeMap<String, serde_json::Value>,
}

impl SavedEffect {
    fn capture(effect: &EffectInstance) -> Result<Self> {
        let params = effect
            .params
            .iter()
            .map(|param| {
                let value = match &param.value {
                    ParamValue::Integer(v) => serde_json::to_value(v)?,
                    ParamValue::Float(v) => serde_json::to_value(v)?,
                    ParamValue::Bool(v) => serde_json::to_value(v)?,
                    ParamValue::Choice(v) => serde_json::to_value(v)?,
                    ParamValue::File(file) => serde_json::to_value(
                        file.as_ref()
                            .map(|file| std::path::absolute(&file.path))
                            .transpose()?,
                    )?,
                };
                Ok((param.id.to_owned(), value))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            id: effect.id,
            type_id: effect.type_id.to_owned(),
            enabled: effect.enabled,
            params,
        })
    }

    fn restore(
        self,
        role: EffectRole,
        ids: &mut HashSet<u64>,
        files: &mut Vec<(u64, &'static str, effects::FileKind, PathBuf)>,
    ) -> Result<EffectInstance> {
        ensure!(
            self.id <= 1_000_000_000 && ids.insert(self.id),
            "IDが重複または範囲外です"
        );
        let definition = effects::definitions()
            .iter()
            .find(|d| d.info.id == self.type_id)
            .context("未知のエフェクトです")?;
        ensure!(definition.info.role == role, "エフェクトの配置が不正です");
        ensure!(
            self.params.len() == definition.info.param_definitions.len(),
            "パラメーター数が不正です"
        );
        let mut effect = EffectInstance::new(self.id, definition);
        effect.enabled = self.enabled;
        for (param, def) in effect
            .params
            .iter_mut()
            .zip(definition.info.param_definitions)
        {
            let value = self
                .params
                .get(def.id)
                .context("パラメーターがありません")?
                .clone();
            param.value = match def.default {
                ParamValue::Integer(_) => ParamValue::Integer(serde_json::from_value(value)?),
                ParamValue::Float(_) => ParamValue::Float(serde_json::from_value(value)?),
                ParamValue::Bool(_) => ParamValue::Bool(serde_json::from_value(value)?),
                ParamValue::Choice(_) => {
                    let value: String = serde_json::from_value(value)?;
                    ParamValue::Choice(
                        def.choices
                            .iter()
                            .find(|(id, _)| *id == value)
                            .context("選択値が不正です")?
                            .0,
                    )
                }
                ParamValue::File(_) => {
                    let path: Option<PathBuf> = serde_json::from_value(value)?;
                    if let Some(path) = path {
                        files.push((self.id, def.id, def.file.expect("file kind"), path));
                    }
                    ParamValue::File(None)
                }
            };
            if let Some(range) = def.number {
                let value = param.value.number();
                ensure!(
                    value.is_finite() && (range.min..=range.max).contains(&value),
                    "パラメーターが範囲外です"
                );
            }
        }
        Ok(effect)
    }
}

pub struct Restored {
    pub document: Document,
    pub files: Vec<(u64, &'static str, effects::FileKind, PathBuf)>,
}

impl Project {
    pub fn capture(document: &Document) -> Result<Self> {
        let clips = document
            .clips
            .iter()
            .map(|clip| {
                Ok(SavedClip {
                    id: clip.id,
                    kind: match &clip.kind {
                        ObjectKind::Input { input, output } => SavedKind::Input {
                            input: SavedEffect::capture(input)?,
                            output: SavedEffect::capture(output)?,
                        },
                        ObjectKind::Camera { settings } => SavedKind::Camera {
                            settings: SavedEffect::capture(settings)?,
                        },
                        ObjectKind::Filter => SavedKind::Filter,
                        ObjectKind::Effect => SavedKind::Effect,
                    },
                    effects: clip
                        .effects
                        .iter()
                        .map(SavedEffect::capture)
                        .collect::<Result<_>>()?,
                    layer: clip.layer,
                    start: clip.start,
                    length: clip.length,
                    source_start: clip.source_start,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            version: 1,
            settings: document.settings,
            clips,
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(path.parent().context("保存先が不正です")?)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        ensure!(
            file.as_file().metadata()?.len() <= 16 * 1024 * 1024,
            "プロジェクトが16MiBを超えています"
        );
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Restored> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "プロジェクトが16MiBを超えています"
        );
        let project: Self = serde_json::from_slice(&bytes)?;
        ensure!(project.version == 1, "未対応のプロジェクト形式です");
        project.settings.validate()?;
        let mut ids = HashSet::new();
        let mut files = Vec::new();
        let mut clips = Vec::new();
        for clip in project.clips {
            ensure!(
                clip.id <= 1_000_000_000 && ids.insert(clip.id),
                "IDが重複または範囲外です"
            );
            ensure!(
                clip.layer < 1_000_000
                    && clip.length > 0
                    && clip
                        .start
                        .checked_add(clip.length)
                        .is_some_and(|end| end <= 1_000_000_000)
                    && clip.source_start.checked_add(clip.length).is_some(),
                "クリップ範囲が不正です"
            );
            let kind = match clip.kind {
                SavedKind::Input { input, output } => ObjectKind::Input {
                    input: input.restore(EffectRole::Input, &mut ids, &mut files)?,
                    output: output.restore(EffectRole::Output, &mut ids, &mut files)?,
                },
                SavedKind::Camera { settings } => ObjectKind::Camera {
                    settings: settings.restore(EffectRole::Camera, &mut ids, &mut files)?,
                },
                SavedKind::Filter => ObjectKind::Filter,
                SavedKind::Effect => ObjectKind::Effect,
            };
            clips.push(Clip {
                id: clip.id,
                kind,
                effects: clip
                    .effects
                    .into_iter()
                    .map(|e| e.restore(EffectRole::Effect, &mut ids, &mut files))
                    .collect::<Result<_>>()?,
                layer: clip.layer,
                start: clip.start,
                length: clip.length,
                source_start: clip.source_start,
            });
        }
        clips.sort_by_key(|clip| (clip.layer, clip.start));
        ensure!(
            clips
                .windows(2)
                .all(|pair| pair[0].layer != pair[1].layer || pair[0].end() <= pair[1].start),
            "同一レイヤーでクリップが重複しています"
        );
        for (_, _, _, file) in &mut files {
            if file.is_relative() {
                *file = path
                    .parent()
                    .context("プロジェクトの親フォルダーがありません")?
                    .join(&*file);
            }
        }
        Ok(Restored {
            document: Document {
                settings: project.settings,
                clips,
            },
            files,
        })
    }
}
