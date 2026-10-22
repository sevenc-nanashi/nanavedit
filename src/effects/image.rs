use super::*;

pub(super) fn definition() -> EffectDefinition {
    EffectDefinition {
        info: EffectInfo {
            id: "builtin.image",
            name: "画像",
            role: EffectRole::Input,
            param_definitions: &[],
            initialization: Initialization::ImageFile,
            time_dependent: false,
        },
        #[cfg(target_os = "windows")]
        create_processor: Some(|_| Ok(Box::new(Image))),
    }
}

#[cfg(target_os = "windows")]
struct Image;

#[cfg(target_os = "windows")]
impl EffectProcessor for Image {
    fn process(
        &mut self,
        _source: Option<Source>,
        _params: &[Param],
        _render_params: &mut RenderParams,
        context: &mut ProcessContext<'_>,
    ) -> anyhow::Result<Source> {
        context.image(0)
    }
}
