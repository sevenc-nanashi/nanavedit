use super::*;

pub(super) const fn number(
    id: &'static str,
    name: &'static str,
    value: f32,
    range: (f32, f32),
    precision: usize,
    group: &'static str,
) -> ParamDefinition {
    ParamDefinition {
        id,
        name,
        default: ParamValue::Float(value),
        number: Some(NumberDefinition {
            min: range.0,
            max: range.1,
            step: if precision == 3 { 0.001 } else { 0.01 },
            precision,
            unit: "",
        }),
        choices: &[],
        group: Some(group),
    }
}

pub(super) fn definition() -> EffectDefinition {
    EffectDefinition {
        info: EffectInfo {
            id: "builtin.standard_draw",
            name: "標準描画",
            role: EffectRole::Output,
            param_definitions: const {
                &[
                    number("x", "X", 0.0, (-10000.0, 10000.0), 2, "位置"),
                    number("y", "Y", 0.0, (-10000.0, 10000.0), 2, "位置"),
                    number("z", "Z", 0.0, (-10000.0, 10000.0), 2, "位置"),
                    number("cx", "中心X", 0.0, (-10000.0, 10000.0), 2, "中心"),
                    number("cy", "中心Y", 0.0, (-10000.0, 10000.0), 2, "中心"),
                    number("cz", "中心Z", 0.0, (-10000.0, 10000.0), 2, "中心"),
                    number("rx", "X軸回転", 0.0, (-36000.0, 36000.0), 2, "回転"),
                    number("ry", "Y軸回転", 0.0, (-36000.0, 36000.0), 2, "回転"),
                    number("rz", "Z軸回転", 0.0, (-36000.0, 36000.0), 2, "回転"),
                    number("zoom", "拡大率", 100.0, (0.0, 10000.0), 3, "描画"),
                    number("aspect", "縦横比", 0.0, (-100.0, 100.0), 3, "描画"),
                    number("transparency", "透明度", 0.0, (0.0, 100.0), 2, "描画"),
                    ParamDefinition {
                        id: "blend",
                        name: "合成モード",
                        default: ParamValue::Choice("normal"),
                        number: None,
                        choices: &[("normal", "通常")],
                        group: Some("描画"),
                    },
                    ParamDefinition {
                        id: "camera",
                        name: "カメラ制御の対象",
                        default: ParamValue::Bool(true),
                        number: None,
                        choices: &[],
                        group: Some("描画"),
                    },
                ]
            },
            initialization: Initialization::Defaults,
            time_dependent: false,
        },
        #[cfg(target_os = "windows")]
        create_processor: Some(|_| Ok(Box::new(StandardDraw))),
    }
}

#[cfg(target_os = "windows")]
struct StandardDraw;

#[cfg(target_os = "windows")]
impl EffectProcessor for StandardDraw {
    fn process(
        &mut self,
        source: Option<Source>,
        params: &[Param],
        render: &mut RenderParams,
        _context: &mut ProcessContext<'_>,
    ) -> anyhow::Result<Source> {
        render.position = ["x", "y", "z"].map(|id| param(params, id).number());
        render.center = ["cx", "cy", "cz"].map(|id| param(params, id).number());
        render.rotation = ["rx", "ry", "rz"].map(|id| param(params, id).number());
        let zoom = param(params, "zoom").number() / 100.0;
        let aspect = param(params, "aspect").number() / 100.0;
        render.scale = [
            zoom * (1.0 - aspect.max(0.0)),
            zoom * (1.0 + aspect.min(0.0)),
            zoom,
        ];
        render.alpha *= 1.0 - param(params, "transparency").number() / 100.0;
        render.camera_enabled = param(params, "camera") == ParamValue::Bool(true);
        render.blend = match param(params, "blend") {
            ParamValue::Choice("normal") => crate::preview::effects::BlendMode::Normal,
            _ => unreachable!("registered blend mode"),
        };
        source.ok_or_else(|| anyhow::anyhow!("標準描画の入力画像がありません"))
    }
}
