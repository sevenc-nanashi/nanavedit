use super::*;

pub(super) fn definition() -> EffectDefinition {
    EffectDefinition {
        info: EffectInfo {
            id: "builtin.opacity",
            name: "不透明度",
            role: EffectRole::Effect,
            param_definitions: &[ParamDefinition {
                id: "opacity",
                name: "不透明度",
                default: ParamValue::Integer(100),
                number: Some(NumberDefinition {
                    min: 0.0,
                    max: 100.0,
                    step: 1.0,
                    precision: 0,
                    unit: "%",
                }),
            }],
            initialization: Initialization::Defaults,
            time_dependent: false,
        },
        #[cfg(target_os = "windows")]
        create_processor: |_| Ok(Box::new(Opacity)),
    }
}

#[cfg(target_os = "windows")]
struct Opacity;

#[cfg(target_os = "windows")]
impl EffectProcessor for Opacity {
    fn process(
        &mut self,
        source: Option<Source>,
        params: &[Param],
        render_params: &mut RenderParams,
        _context: &mut ProcessContext<'_>,
    ) -> anyhow::Result<Source> {
        render_params.alpha *= param(params, "opacity").number() / 100.0;
        source.ok_or_else(|| anyhow::anyhow!("不透明度の入力画像がありません"))
    }
}
