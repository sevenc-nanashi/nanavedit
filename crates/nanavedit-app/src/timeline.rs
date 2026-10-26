use crate::effects::EffectInstance;
pub use nanavedit_engine::timeline::{Clip, Document, ImageAsset, ObjectKind, ProjectSettings};
use std::{collections::HashSet, time::Instant};
const HISTORY_LIMIT: usize = 100;

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
            document: Document::default(),
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
    pub fn replace_document(&mut self, document: Document) {
        self.next_id = self.next_id.max(
            document
                .clips
                .iter()
                .flat_map(|clip| {
                    std::iter::once(clip.id).chain(clip.instances().map(|effect| effect.id))
                })
                .max()
                .map_or(0, |id| id + 1),
        );
        self.document = document;
        self.undo.clear();
        self.redo.clear();
        self.selected = None;
        self.frame = 0;
        self.layer = 0;
        self.playing = None;
    }

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
            if start + (self.document.settings.fps * 5) <= clip.start {
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
            length: self.document.settings.fps * 5,
            source_start: 0,
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

    pub fn nearest_start(&self, clip: &Clip) -> Option<u64> {
        let mut occupied: Vec<_> = self
            .document
            .clips
            .iter()
            .filter(|other| other.id != clip.id && other.layer == clip.layer)
            .map(|other| (other.start, other.end()))
            .collect();
        occupied.sort_unstable();
        let mut gap_start = 0;
        let mut nearest = None;
        for (start, end) in occupied.into_iter().chain([(u64::MAX, u64::MAX)]) {
            if start - gap_start >= clip.length {
                let candidate = clip.start.clamp(gap_start, start - clip.length);
                let key = (candidate.abs_diff(clip.start), candidate);
                if nearest.is_none_or(|best| key < best) {
                    nearest = Some(key);
                }
            }
            gap_start = end;
        }
        nearest.map(|(_, start)| start)
    }

    pub fn resize_bounds(&self, original: &Clip) -> (u64, u64) {
        let mut start = 0;
        let mut end = u64::MAX;
        for other in &self.document.clips {
            if other.id == original.id || other.layer != original.layer {
                continue;
            }
            if other.end() <= original.start {
                start = start.max(other.end());
            } else if other.start >= original.end() {
                end = end.min(other.start);
            }
        }
        if let Some(video) = original.video() {
            start = start.max(original.start.saturating_sub(original.source_start));
            end = end.min(original.start.saturating_add(
                video.duration(self.document.settings.fps) - original.source_start,
            ));
        }
        (start, end)
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
        self.document.active_objects(self.frame)
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
                    .filter_map(|file| file.resource.image().map(|image| image.id))
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
            let frame = start
                + (time.elapsed().as_secs_f64() * self.document.settings.fps as f64).floor() as u64;
            let last = self.end().saturating_sub(1);
            self.frame = frame.min(last);
            if frame >= last {
                self.playing = None;
            }
        }
    }
}
