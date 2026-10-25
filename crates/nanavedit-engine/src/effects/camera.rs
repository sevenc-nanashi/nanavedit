use super::standard_draw::number;
use super::*;

pub(super) fn definition() -> EffectDefinition {
    EffectDefinition {
        info: EffectInfo {
            id: "builtin.camera",
            name: "カメラ制御",
            role: EffectRole::Camera,
            param_definitions: const {
                &[
                    number("x", "X", 0.0, (-10000.0, 10000.0), 2, "位置"),
                    number("y", "Y", 0.0, (-10000.0, 10000.0), 2, "位置"),
                    number("z", "Z", -1024.0, (-10000.0, 10000.0), 2, "位置"),
                    number("tx", "目標X", 0.0, (-10000.0, 10000.0), 2, "目標"),
                    number("ty", "目標Y", 0.0, (-10000.0, 10000.0), 2, "目標"),
                    number("tz", "目標Z", 0.0, (-10000.0, 10000.0), 2, "目標"),
                    number("roll", "傾き", 0.0, (-36000.0, 36000.0), 2, "投影"),
                    number("fov", "視野角（0: 既定値）", 0.0, (0.0, 179.0), 2, "投影"),
                    ParamDefinition {
                        id: "layers",
                        name: "対象レイヤー数（0: 制限なし）",
                        default: ParamValue::Integer(0),
                        number: Some(NumberDefinition {
                            min: 0.0,
                            max: 10000.0,
                            step: 1.0,
                            precision: 0,
                            unit: "",
                        }),
                        file: None,
                        choices: &[],
                        group: Some("適用範囲"),
                    },
                ]
            },

            time_dependent: false,
        },
        #[cfg(target_os = "windows")]
        create_processor: None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CameraParams {
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub roll: f32,
    pub fov: f32,
    pub layers: usize,
}

impl CameraParams {
    pub fn evaluate(params: &[Param]) -> Self {
        Self {
            position: ["x", "y", "z"].map(|id| param(params, id).number()),
            target: ["tx", "ty", "tz"].map(|id| param(params, id).number()),
            roll: param(params, "roll").number(),
            fov: param(params, "fov").number(),
            layers: match param(params, "layers") {
                ParamValue::Integer(value) => value as usize,
                _ => unreachable!("integer layer count"),
            },
        }
    }
}
