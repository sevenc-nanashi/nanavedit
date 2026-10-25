use super::*;

pub(super) fn definition() -> EffectDefinition {
    EffectDefinition {
        info: EffectInfo {
            id: "builtin.image",
            name: "画像ファイルオブジェクト",
            role: EffectRole::Input,
            param_definitions: &[ParamDefinition {
                id: "file",
                name: "ファイル",
                default: ParamValue::File(None),
                number: None,
                file: Some(FileKind::Image),
                choices: &[],
                group: None,
            }],
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
        params: &[Param],
        _render_params: &mut RenderParams,
        context: &mut ProcessContext<'_>,
    ) -> anyhow::Result<Source> {
        match param(params, "file") {
            ParamValue::File(Some(file)) => context.image(&file),
            ParamValue::File(None) => context.transparent_image(),
            _ => unreachable!("declared file parameter"),
        }
    }
}
