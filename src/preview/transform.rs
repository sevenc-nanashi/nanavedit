use super::{CameraParams, RenderItem, RenderParams, SCENE_SIZE};

const DEFAULT_DISTANCE: f32 = 1024.0;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn local_point(point: [f32; 3], params: &RenderParams) -> [f32; 3] {
    let [x, y, z] = std::array::from_fn(|i| (point[i] - params.center[i]) * params.scale[i]);
    let [(sx, cx), (sy, cy), (sz, cz)] = params.rotation.map(|v| v.to_radians().sin_cos());
    // AviUtl2 applies Z, Y, X rotations after moving/scaling the local origin.
    let (x, y) = (x * cz - y * sz, x * sz + y * cz);
    let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
    [x, y * cx - z * sx, y * sx + z * cx]
}

pub(super) struct Projection {
    camera: Option<CameraParams>,
    right: [f32; 3],
    down: [f32; 3],
    forward: [f32; 3],
    distance: f32,
}

impl Projection {
    pub fn new(camera: Option<CameraParams>) -> Option<Self> {
        let mut result = Self {
            camera,
            right: [1.0, 0.0, 0.0],
            down: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            distance: DEFAULT_DISTANCE,
        };
        if let Some(camera) = camera {
            let [x, y, z] = std::array::from_fn(|i| camera.target[i] - camera.position[i]);
            let horizontal = x.hypot(z);
            // The reference does not draw a camera looking exactly along the Y axis.
            if horizontal == 0.0 && y != 0.0 {
                return None;
            }
            let (sy, cy) = x.atan2(z).sin_cos();
            let (sp, cp) = y.atan2(horizontal).sin_cos();
            result.right = [cy, 0.0, -sy];
            result.down = [-sy * sp, cp, -cy * sp];
            result.forward = [sy * cp, sp, cy * cp];
            if camera.fov != 0.0 {
                result.distance =
                    SCENE_SIZE.1 as f32 / (2.0 * (camera.fov.to_radians() / 2.0).tan());
            }
        }
        Some(result)
    }

    fn view_point(&self, local: [f32; 3], params: &RenderParams) -> [f32; 3] {
        if let Some(camera) = self.camera {
            let relative =
                std::array::from_fn(|i| local[i] + params.position[i] - camera.position[i]);
            let x = dot(relative, self.right);
            let y = dot(relative, self.down);
            let (s, c) = camera.roll.to_radians().sin_cos();
            [x * c - y * s, x * s + y * c, dot(relative, self.forward)]
        } else {
            [
                local[0],
                local[1],
                local[2] + params.position[2] + self.distance,
            ]
        }
    }

    pub fn corners(&self, width: u32, height: u32, params: &RenderParams) -> [[f32; 4]; 4] {
        [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)].map(|(u, v)| {
            let local = local_point([u * width as f32, v * height as f32, 0.0], params);
            let [mut x, mut y, w] = self.view_point(local, params);
            x *= self.distance;
            y *= self.distance;
            // Ordinary drawing translates X/Y in screen space; camera drawing uses world space.
            if self.camera.is_none() {
                x += params.position[0] * w;
                y += params.position[1] * w;
            }
            [
                2.0 * x / SCENE_SIZE.0 as f32,
                -2.0 * y / SCENE_SIZE.1 as f32,
                w - self.distance * 0.05,
                w,
            ]
        })
    }

    pub fn clip(&self, point: [f32; 3]) -> [f32; 4] {
        [
            2.0 * point[0] * self.distance / SCENE_SIZE.0 as f32,
            -2.0 * point[1] * self.distance / SCENE_SIZE.1 as f32,
            point[2] - self.distance * 0.05,
            point[2],
        ]
    }
}

#[derive(Clone, Copy)]
pub(super) struct Vertex {
    pub point: [f32; 3],
    pub uv: [f32; 2],
}

pub(super) struct Polygon {
    pub item: usize,
    pub vertices: Vec<Vertex>,
}

struct Node {
    normal: [f32; 3],
    offset: f32,
    polygons: Vec<Polygon>,
    children: [Option<usize>; 2],
}

impl Node {
    fn new(polygon: Polygon) -> Option<Self> {
        let origin = polygon.vertices[0].point;
        let a: [f32; 3] = std::array::from_fn(|i| polygon.vertices[1].point[i] - origin[i]);
        let b: [f32; 3] =
            std::array::from_fn(|i| polygon.vertices.last().expect("polygon").point[i] - origin[i]);
        let normal = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = dot(normal, normal).sqrt();
        if length == 0.0 {
            return None;
        }
        let normal = normal.map(|v| v / length);
        Some(Self {
            normal,
            offset: dot(normal, origin),
            polygons: vec![polygon],
            children: [None, None],
        })
    }

    fn split(&self, polygon: Polygon) -> (Option<Polygon>, [Option<Polygon>; 2]) {
        const EPSILON: f32 = 0.0001;
        let distances: Vec<_> = polygon
            .vertices
            .iter()
            .map(|v| dot(self.normal, v.point) - self.offset)
            .collect();
        let back = distances.iter().any(|d| *d < -EPSILON);
        let front = distances.iter().any(|d| *d > EPSILON);
        match (back, front) {
            (false, false) => return (Some(polygon), [None, None]),
            (true, false) => return (None, [Some(polygon), None]),
            (false, true) => return (None, [None, Some(polygon)]),
            _ => {}
        }
        let mut sides = [Vec::new(), Vec::new()];
        for i in 0..polygon.vertices.len() {
            let j = (i + 1) % polygon.vertices.len();
            let a = polygon.vertices[i];
            let b = polygon.vertices[j];
            let (da, db) = (distances[i], distances[j]);
            if da <= EPSILON {
                sides[0].push(a);
            }
            if da >= -EPSILON {
                sides[1].push(a);
            }
            if (da < -EPSILON && db > EPSILON) || (da > EPSILON && db < -EPSILON) {
                let t = da / (da - db);
                let cut = Vertex {
                    point: std::array::from_fn(|k| a.point[k] + t * (b.point[k] - a.point[k])),
                    uv: std::array::from_fn(|k| a.uv[k] + t * (b.uv[k] - a.uv[k])),
                };
                sides[0].push(cut);
                sides[1].push(cut);
            }
        }
        (
            None,
            sides.map(|vertices| {
                (vertices.len() >= 3).then_some(Polygon {
                    item: polygon.item,
                    vertices,
                })
            }),
        )
    }
}

// Split intersecting planes so translucent pixels can be composited back to front too.
pub(super) fn camera_polygons(items: &[RenderItem], projection: &Projection) -> Vec<Polygon> {
    let mut nodes: Vec<Node> = Vec::new();
    for (item, source) in items.iter().enumerate() {
        if !source.params.camera_enabled {
            continue;
        }
        let vertices = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
            .map(|uv| {
                let local = local_point(
                    [
                        (uv[0] - 0.5) * source.source.width as f32,
                        (uv[1] - 0.5) * source.source.height as f32,
                        0.0,
                    ],
                    &source.params,
                );
                Vertex {
                    point: projection.view_point(local, &source.params),
                    uv,
                }
            })
            .to_vec();
        let polygon = Polygon { item, vertices };
        if nodes.is_empty() {
            if let Some(node) = Node::new(polygon) {
                nodes.push(node);
            }
            continue;
        }
        let mut pending = vec![(0, polygon)];
        while let Some((index, polygon)) = pending.pop() {
            let (coplanar, sides) = nodes[index].split(polygon);
            nodes[index].polygons.extend(coplanar);
            for (side, polygon) in sides.into_iter().enumerate() {
                if let Some(polygon) = polygon {
                    if let Some(child) = nodes[index].children[side] {
                        pending.push((child, polygon));
                    } else if let Some(node) = Node::new(polygon) {
                        nodes[index].children[side] = Some(nodes.len());
                        nodes.push(node);
                    }
                }
            }
        }
    }
    let mut ordered = Vec::new();
    if nodes.is_empty() {
        return ordered;
    }
    let mut pending = vec![(0, false)];
    while let Some((index, visit)) = pending.pop() {
        if visit {
            ordered.append(&mut nodes[index].polygons);
            continue;
        }
        let near = usize::from(nodes[index].offset <= 0.0);
        if let Some(child) = nodes[index].children[near] {
            pending.push((child, false));
        }
        pending.push((index, true));
        if let Some(child) = nodes[index].children[1 - near] {
            pending.push((child, false));
        }
    }
    ordered
}
