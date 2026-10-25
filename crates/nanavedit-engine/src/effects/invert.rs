use super::*;

pub(super) fn definition() -> EffectDefinition {
    EffectDefinition {
        info: EffectInfo {
            id: "builtin.invert",
            name: "色反転",
            role: EffectRole::Effect,
            param_definitions: &[],

            time_dependent: false,
        },
        #[cfg(target_os = "windows")]
        create_processor: Some(|context| {
            Ok(Box::new(Invert {
                pipeline: context.create_pipeline("invert", include_str!("invert.wgsl"))?,
            }))
        }),
    }
}

#[cfg(target_os = "windows")]
struct Invert {
    pipeline: wgpu::RenderPipeline,
}

#[cfg(target_os = "windows")]
impl EffectProcessor for Invert {
    fn process(
        &mut self,
        source: Option<Source>,
        _params: &[Param],
        _render_params: &mut RenderParams,
        context: &mut ProcessContext<'_>,
    ) -> anyhow::Result<Source> {
        let source = source.ok_or_else(|| anyhow::anyhow!("色反転の入力画像がありません"))?;
        let target = context.acquire(source.width(), source.height())?;
        let mut uniform = [0u8; 16];
        uniform[..4].copy_from_slice(
            &(if source.premultiplied() {
                1.0f32
            } else {
                0.0f32
            })
            .to_le_bytes(),
        );
        context.render(&source, &target, &self.pipeline, &uniform)?;
        context.release(source);
        Ok(target)
    }
}
