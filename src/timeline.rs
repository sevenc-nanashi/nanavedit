use crate::effects::EffectInstance;
use std::{collections::HashSet, time::Instant};

pub const FPS: u64 = 30;
pub const INITIAL_LENGTH: u64 = FPS * 5;
const HISTORY_LIMIT: usize = 100;

#[derive(Debug, PartialEq, Eq)]
pub struct ImageAsset {
    pub id: u64,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

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
}

impl Clip {
    pub fn image(&self) -> Option<&ImageAsset> {
        match &self.kind {
            ObjectKind::Input { input, .. } => {
                input.files().next().map(|file| file.resource.image())
            }
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match &self.kind {
            ObjectKind::Input { input, .. } => match self.image() {
                Some(image) => &image.name,
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

pub struct Timeline {
    pub document: Document,
    pub selected: Option<u64>,
    pub layer: usize,
    pub frame: u64,
    pub playing: Option<(Instant, u64)>,
    pub zoom: f32,
    undo: Vec<Document>,
    redo: Vec<Document>,
    next_id: u64,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            document: Document { clips: Vec::new() },
            selected: None,
            layer: 0,
            frame: 0,
            playing: None,
            zoom: 2.0,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
        }
    }
}

impl Timeline {
    pub fn selected_clip(&self) -> Option<&Clip> {
        self.document
            .clips
            .iter()
            .find(|clip| Some(clip.id) == self.selected)
    }

    pub fn end(&self) -> u64 {
        self.document.clips.iter().map(Clip::end).max().unwrap_or(0)
    }

    pub fn commit(&mut self, before: Document) {
        if before != self.document {
            self.undo.push(before);
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, redo: bool) {
        self.playing = None;
        let (source, target) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(document) = source.pop() {
            target.push(std::mem::replace(&mut self.document, document));
            if let Some(clip) = self.selected_clip() {
                self.layer = clip.layer;
            } else {
                self.selected = None;
            }
        }
    }

    pub fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add_object(&mut self, kind: ObjectKind, layer: usize, frame: u64) {
        let before = self.document.clone();
        let mut start = frame;
        let mut clips: Vec<_> = self
            .document
            .clips
            .iter()
            .filter(|c| c.layer == layer)
            .collect();
        clips.sort_by_key(|c| c.start);
        for clip in clips {
            if start + INITIAL_LENGTH <= clip.start {
                break;
            }
            if start < clip.end() {
                start = clip.end();
            }
        }
        let id = self.allocate_id();
        self.document.clips.push(Clip {
            id,
            kind,
            effects: Vec::new(),
            layer,
            start,
            length: INITIAL_LENGTH,
        });
        self.selected = Some(id);
        self.layer = layer;
        self.frame = start;
        self.commit(before);
    }

    pub fn delete_selected(&mut self) {
        let before = self.document.clone();
        self.document.clips.retain(|c| Some(c.id) != self.selected);
        self.selected = None;
        self.commit(before);
    }

    pub fn place(&mut self, clip: Clip) -> bool {
        if clip.length == 0
            || self.document.clips.iter().any(|other| {
                other.id != clip.id
                    && other.layer == clip.layer
                    && clip.start < other.end()
                    && other.start < clip.end()
            })
        {
            return false;
        }
        let current = self
            .document
            .clips
            .iter_mut()
            .find(|c| c.id == clip.id)
            .expect("dragged clip exists");
        self.layer = clip.layer;
        *current = clip;
        true
    }

    pub fn active_objects(&self) -> Vec<Clip> {
        let mut clips: Vec<_> = self
            .document
            .clips
            .iter()
            .filter(|c| c.start <= self.frame && self.frame < c.end())
            .collect();
        clips.sort_by_key(|c| c.layer);
        clips.into_iter().cloned().collect()
    }

    pub fn effect_target(&self, id: u64) -> Option<u64> {
        let mut target = None;
        for clip in self.active_objects() {
            if clip.id == id {
                return target;
            }
            match clip.kind {
                ObjectKind::Input { .. } => target = Some(clip.id),
                ObjectKind::Filter => target = None,
                ObjectKind::Effect | ObjectKind::Camera { .. } => {}
            }
        }
        None
    }

    pub fn retained_images(&self) -> HashSet<u64> {
        std::iter::once(&self.document)
            .chain(&self.undo)
            .chain(&self.redo)
            .flat_map(|document| {
                document
                    .clips
                    .iter()
                    .flat_map(Clip::instances)
                    .flat_map(EffectInstance::files)
                    .map(|file| file.resource.image().id)
            })
            .collect()
    }

    pub fn toggle_playback(&mut self) {
        if self.playing.take().is_none() && self.end() > 0 {
            if self.frame >= self.end() - 1 {
                self.frame = 0;
            }
            self.playing = Some((Instant::now(), self.frame));
        }
    }

    pub fn tick(&mut self) {
        if let Some((time, start)) = self.playing {
            let frame = start + (time.elapsed().as_secs_f64() * FPS as f64).floor() as u64;
            let last = self.end().saturating_sub(1);
            self.frame = frame.min(last);
            if frame >= last {
                self.playing = None;
            }
        }
    }
}
