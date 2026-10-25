use super::*;

pub(super) fn definition() -> EffectDefinition {
    EffectDefinition {
        info: EffectInfo {
            id: "builtin.video",
            name: "動画ファイルオブジェクト",
            role: EffectRole::Input,
            param_definitions: &[ParamDefinition {
                id: "file",
                name: "ファイル",
                default: ParamValue::File(None),
                number: None,
                file: Some(FileKind::Video),
                choices: &[],
                group: None,
            }],
            time_dependent: true,
        },
        #[cfg(target_os = "windows")]
        create_processor: Some(|_| Ok(Box::new(Video))),
    }
}

#[cfg(target_os = "windows")]
struct Video;
#[cfg(target_os = "windows")]
impl EffectProcessor for Video {
    fn process(
        &mut self,
        _source: Option<Source>,
        params: &[Param],
        _render: &mut RenderParams,
        context: &mut ProcessContext<'_>,
    ) -> anyhow::Result<Source> {
        match param(params, "file") {
            ParamValue::File(Some(file)) => context.video(&file),
            ParamValue::File(None) => context.transparent_image(),
            _ => unreachable!("declared file parameter"),
        }
    }
}
