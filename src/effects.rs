use crate::timeline::ImageAsset;
use std::{collections::HashSet, rc::Rc, sync::LazyLock};

mod image;
mod invert;
mod opacity;

#[cfg(target_os = "windows")]
pub use crate::preview::effects::{EffectProcessor, ProcessContext, RenderParams, Source};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Integer(i32),
    Float(f32),
    Bool(bool),
}

impl ParamValue {
    pub fn number(self) -> f32 {
        match self {
            Self::Integer(value) => value as f32,
            Self::Float(value) => value,
            Self::Bool(_) => panic!("boolean parameter is not numeric"),
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
}

impl ParamDefinition {
    pub fn slider_value(&self, value: f32) -> ParamValue {
        let number = self.number.expect("numeric parameter has a range");
        let value = (number.min + ((value - number.min) / number.step).round() * number.step)
            .clamp(number.min, number.max);
        match self.default {
            ParamValue::Integer(_) => ParamValue::Integer(value.round() as i32),
            ParamValue::Float(_) => ParamValue::Float(value),
            ParamValue::Bool(_) => unreachable!("numeric parameter"),
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectRole {
    Input,
    Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Initialization {
    Defaults,
    ImageFile,
}

#[derive(Debug)]
pub struct EffectInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub role: EffectRole,
    pub param_definitions: &'static [ParamDefinition],
    pub initialization: Initialization,
    pub time_dependent: bool,
}

pub struct EffectDefinition {
    pub info: EffectInfo,
    #[cfg(target_os = "windows")]
    pub create_processor: fn(&ProcessContext<'_>) -> anyhow::Result<Box<dyn EffectProcessor>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    Image(Rc<ImageAsset>),
}

impl Resource {
    pub fn image(&self) -> &ImageAsset {
        match self {
            Self::Image(image) => image,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectInstance {
    pub id: u64,
    pub type_id: &'static str,
    pub params: Vec<Param>,
    pub enabled: bool,
    pub resources: Vec<Resource>,
}

impl EffectInstance {
    pub fn new(id: u64, definition: &'static EffectDefinition, resources: Vec<Resource>) -> Self {
        Self {
            id,
            type_id: definition.info.id,
            params: definition
                .info
                .param_definitions
                .iter()
                .map(|p| Param {
                    id: p.id,
                    value: p.default,
                })
                .collect(),
            enabled: true,
            resources,
        }
    }

    pub fn definition(&self) -> &'static EffectDefinition {
        definition(self.type_id)
    }
}

pub fn definitions() -> &'static [EffectDefinition] {
    static REGISTRY: LazyLock<Vec<EffectDefinition>> = LazyLock::new(|| {
        let definitions = vec![
            image::definition(),
            opacity::definition(),
            invert::definition(),
        ];
        let mut ids = HashSet::new();
        for definition in &definitions {
            let info = &definition.info;
            assert!(
                !info.id.is_empty() && ids.insert(info.id),
                "duplicate or empty effect ID: {}",
                info.id
            );
            let mut param_ids = HashSet::new();
            for param in info.param_definitions {
                assert!(
                    !param.id.is_empty() && param_ids.insert(param.id),
                    "duplicate or empty parameter ID: {} / {}",
                    info.id,
                    param.id
                );
                match (param.default, param.number) {
                    (ParamValue::Bool(_), None) => {}
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
