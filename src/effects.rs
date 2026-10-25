use crate::timeline::ImageAsset;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

pub mod camera;
mod image;
mod invert;
mod opacity;
mod standard_draw;
mod video;

#[cfg(target_os = "windows")]
pub use crate::preview::effects::{EffectProcessor, ProcessContext, RenderParams, Source};

#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Integer(i32),
    Float(f32),
    Bool(bool),
    Choice(&'static str),
    File(Option<Arc<FileAsset>>),
}

impl ParamValue {
    pub fn number(&self) -> f32 {
        match self {
            Self::Integer(value) => *value as f32,
            Self::Float(value) => *value,
            _ => panic!("parameter is not numeric"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NumberDefinition {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub precision: usize,
    pub unit: &'static str,
}

#[derive(Debug)]
pub struct ParamDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub default: ParamValue,
    pub number: Option<NumberDefinition>,
    pub file: Option<FileKind>,
    pub choices: &'static [(&'static str, &'static str)],
    pub group: Option<&'static str>,
}

impl ParamDefinition {
    pub fn slider_value(&self, value: f32) -> ParamValue {
        let number = self.number.expect("numeric parameter has a range");
        let value = (number.min + ((value - number.min) / number.step).round() * number.step)
            .clamp(number.min, number.max);
        match self.default {
            ParamValue::Integer(_) => ParamValue::Integer(value.round() as i32),
            ParamValue::Float(_) => ParamValue::Float(value),
            _ => unreachable!("numeric parameter"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub id: &'static str,
    pub value: ParamValue,
}

pub fn param(params: &[Param], id: &str) -> ParamValue {
    params
        .iter()
        .find(|p| p.id == id)
        .expect("parameter is declared by the effect")
        .value
        .clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectRole {
    Input,
    Effect,
    Output,
    Camera,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Video,
}

#[derive(Debug)]
pub struct EffectInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub role: EffectRole,
    pub param_definitions: &'static [ParamDefinition],
    pub time_dependent: bool,
}

pub struct EffectDefinition {
    pub info: EffectInfo,
    #[cfg(target_os = "windows")]
    pub create_processor: Option<ProcessorFactory>,
}

#[cfg(target_os = "windows")]
pub type ProcessorFactory = fn(&ProcessContext<'_>) -> anyhow::Result<Box<dyn EffectProcessor>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    Image(Arc<ImageAsset>),
    Video(Arc<crate::media::VideoAsset>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAsset {
    pub path: PathBuf,
    pub resource: Resource,
}

impl Resource {
    pub fn image(&self) -> Option<&ImageAsset> {
        match self {
            Self::Image(image) => Some(image),
            Self::Video(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectInstance {
    pub id: u64,
    pub type_id: &'static str,
    pub params: Vec<Param>,
    pub enabled: bool,
}

impl EffectInstance {
    pub fn new(id: u64, definition: &'static EffectDefinition) -> Self {
        Self {
            id,
            type_id: definition.info.id,
            params: definition
                .info
                .param_definitions
                .iter()
                .map(|p| Param {
                    id: p.id,
                    value: p.default.clone(),
                })
                .collect(),
            enabled: true,
        }
    }

    pub fn definition(&self) -> &'static EffectDefinition {
        definition(self.type_id)
    }

    pub fn files(&self) -> impl Iterator<Item = &FileAsset> {
        self.params.iter().filter_map(|param| match &param.value {
            ParamValue::File(Some(file)) => Some(file.as_ref()),
            _ => None,
        })
    }
}

pub fn definitions() -> &'static [EffectDefinition] {
    static REGISTRY: LazyLock<Vec<EffectDefinition>> = LazyLock::new(|| {
        let definitions = vec![
            image::definition(),
            video::definition(),
            opacity::definition(),
            invert::definition(),
            standard_draw::definition(),
            camera::definition(),
        ];
        let mut ids = HashSet::new();
        for definition in &definitions {
            let info = &definition.info;
            #[cfg(target_os = "windows")]
            assert_eq!(
                definition.create_processor.is_none(),
                info.role == EffectRole::Camera,
                "only camera settings use the separate camera evaluator"
            );
            assert!(
                !info.id.is_empty() && ids.insert(info.id),
                "duplicate or empty effect ID: {}",
                info.id
            );
            let mut param_ids = HashSet::new();
            for param in info.param_definitions {
                assert_eq!(
                    matches!(param.default, ParamValue::File(_)),
                    param.file.is_some(),
                    "file parameters require a file kind"
                );
                assert!(
                    matches!(param.default, ParamValue::Choice(_)) || param.choices.is_empty(),
                    "choices require a choice parameter"
                );
                assert!(
                    !param.id.is_empty() && param_ids.insert(param.id),
                    "duplicate or empty parameter ID: {} / {}",
                    info.id,
                    param.id
                );
                match (&param.default, param.number) {
                    (ParamValue::Bool(_), None) => {}
                    (ParamValue::File(None), None) => {}
                    (ParamValue::Choice(value), None) => {
                        let mut choices = HashSet::new();
                        assert!(
                            param
                                .choices
                                .iter()
                                .all(|(id, _)| !id.is_empty() && choices.insert(*id))
                        );
                        assert!(choices.contains(value), "choice default must be declared");
                    }
                    (ParamValue::Integer(_) | ParamValue::Float(_), Some(number)) => {
                        assert!(
                            number.min.is_finite()
                                && number.max.is_finite()
                                && number.min < number.max
                                && number.step.is_finite()
                                && number.step > 0.0
                                && ((number.max - number.min) / number.step).is_finite()
                                && number.min + number.step > number.min
                                && number.max - number.step < number.max
                                && number.precision <= 9,
                            "invalid numeric definition: {} / {}",
                            info.id,
                            param.id
                        );
                        let value = param.default.number();
                        assert!(
                            value.is_finite() && (number.min..=number.max).contains(&value),
                            "default is outside parameter range: {} / {}",
                            info.id,
                            param.id
                        );
                        if matches!(param.default, ParamValue::Integer(_)) {
                            // Slider uses f32; reject integer ranges it cannot represent exactly.
                            assert!(
                                number.min >= -16_777_216.0
                                    && number.max <= 16_777_216.0
                                    && number.min.fract() == 0.0
                                    && number.max.fract() == 0.0
                                    && number.step.fract() == 0.0
                                    && number.precision == 0,
                                "invalid integer slider range: {} / {}",
                                info.id,
                                param.id
                            );
                        }
                    }
                    _ => panic!("parameter type/range mismatch: {} / {}", info.id, param.id),
                }
            }
        }
        definitions
    });
    &REGISTRY
}

pub fn definition(id: &str) -> &'static EffectDefinition {
    definitions()
        .iter()
        .find(|d| d.info.id == id)
        .expect("effect type is registered")
}
