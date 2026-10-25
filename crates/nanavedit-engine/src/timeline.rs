use crate::effects::EffectInstance;

pub const FPS: u64 = 30;
pub const INITIAL_LENGTH: u64 = FPS * 5;

pub use nanavedit_media::ImageAsset;

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    Input {
        input: EffectInstance,
        output: EffectInstance,
    },
    Camera {
        settings: EffectInstance,
    },
    Filter,
    Effect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: u64,
    pub kind: ObjectKind,
    pub effects: Vec<EffectInstance>,
    pub layer: usize,
    pub start: u64,
    pub length: u64,
    pub source_start: u64,
}

impl Clip {
    pub fn image(&self) -> Option<&ImageAsset> {
        match &self.kind {
            ObjectKind::Input { input, .. } => {
                input.files().next().and_then(|file| file.resource.image())
            }
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match &self.kind {
            ObjectKind::Input { input, .. } => match input.files().next() {
                Some(file) => match &file.resource {
                    crate::effects::Resource::Image(image) => &image.name,
                    crate::effects::Resource::Video(video) => &video.name,
                },
                None => input.definition().info.name,
            },
            ObjectKind::Filter => "フィルタオブジェクト",
            ObjectKind::Effect => "エフェクトオブジェクト",
            ObjectKind::Camera { .. } => "カメラ制御",
        }
    }

    pub fn instances(&self) -> impl Iterator<Item = &EffectInstance> {
        let fixed = match &self.kind {
            ObjectKind::Input { input, output } => [Some(input), Some(output)],
            ObjectKind::Camera { settings } => [Some(settings), None],
            _ => [None, None],
        };
        fixed.into_iter().flatten().chain(&self.effects)
    }

    pub fn video(&self) -> Option<&std::sync::Arc<crate::media::VideoAsset>> {
        let ObjectKind::Input { input, .. } = &self.kind else {
            return None;
        };
        input.files().find_map(|file| match &file.resource {
            crate::effects::Resource::Video(video) => Some(video),
            _ => None,
        })
    }

    pub fn instance_mut(&mut self, id: u64) -> &mut EffectInstance {
        match &mut self.kind {
            ObjectKind::Input { input, output } => {
                if input.id == id {
                    return input;
                }
                if output.id == id {
                    return output;
                }
            }
            ObjectKind::Camera { settings } if settings.id == id => return settings,
            _ => {}
        }
        self.effects
            .iter_mut()
            .find(|effect| effect.id == id)
            .expect("edited effect exists")
    }

    pub fn end(&self) -> u64 {
        self.start + self.length
    }
}

#[derive(Clone, PartialEq)]
pub struct Document {
    pub clips: Vec<Clip>,
}

impl Document {
    pub fn active_objects(&self, frame: u64) -> Vec<Clip> {
        let mut clips: Vec<_> = self
            .clips
            .iter()
            .filter(|c| c.start <= frame && frame < c.end())
            .collect();
        clips.sort_by_key(|c| c.layer);
        clips.into_iter().cloned().collect()
    }
}
