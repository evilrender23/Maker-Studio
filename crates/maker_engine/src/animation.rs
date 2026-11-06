//! Sistema de animación esquelética: avanza los `Animator`, muestrea sus
//! clips (con fundido entre dos) y escribe la paleta de huesos (`SkinPose`)
//! de cada malla con piel de la entidad y de sus hijos sin Animator propio.
//!
//! Redirección (retargeting) por nombre de hueso, como hace la Universal
//! Animation Library de Quaternius con sus personajes: las rotaciones se
//! copian tal cual; la traslación solo en la cadera (como diferencia respecto
//! al reposo, para respetar la altura de cada personaje) y, con `in_place`, el
//! hueso `root` no se mueve en horizontal (clips `_RM`).

use crate::biblioteca;
use crate::components::*;
use crate::log;
use crate::model::{AnimAsset, ChannelPath, Clip, ModelData, Trs};
use crate::scene::Scene;
use glam::{Mat4, Quat};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

type AssetCache = Mutex<HashMap<PathBuf, Option<Arc<AnimAsset>>>>;

fn cache() -> &'static AssetCache {
    static C: OnceLock<AssetCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Registra el esqueleto y los clips de un modelo ya cargado (el renderizador
/// lo hace al subir la malla, así no se lee dos veces).
pub fn cache_asset(path: &Path, asset: Arc<AnimAsset>) {
    cache().lock().unwrap().insert(path.to_path_buf(), Some(asset));
}

/// Olvida los archivos cargados (tras reimportar).
pub fn clear_cache() {
    cache().lock().unwrap().clear();
}

/// Esqueleto y clips de un archivo (cacheados; `None` si no se puede leer).
pub fn asset(path: &Path) -> Option<Arc<AnimAsset>> {
    if let Some(a) = cache().lock().unwrap().get(path) {
        return a.clone();
    }
    let loaded = match ModelData::load(path) {
        Ok(m) => Some(m.anim),
        Err(e) => {
            log::error("animación", e);
            None
        }
    };
    cache().lock().unwrap().insert(path.to_path_buf(), loaded.clone());
    loaded
}

/// Nombres de los clips de un recurso (`biblioteca://…` o del proyecto).
pub fn clip_names(path: &str, root: Option<&Path>) -> Vec<String> {
    asset(&biblioteca::resolve(path, root)).map(|a| a.clips.iter().map(|c| c.name.clone()).collect()).unwrap_or_default()
}

fn is_hips(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "pelvis" | "hips" | "mixamorig:hips")
}

/// Fuente → destino: para cada nodo de la fuente, el nodo del destino con el
/// mismo nombre.
pub fn retarget_map(target: &AnimAsset, source: &AnimAsset) -> Vec<Option<usize>> {
    let by_name: HashMap<&str, usize> = target.skeleton.nodes.iter().enumerate().map(|(i, n)| (n.name.as_str(), i)).collect();
    source.skeleton.nodes.iter().map(|n| by_name.get(n.name.as_str()).copied()).collect()
}

/// Aplica un clip sobre las locales del destino.
pub fn apply_clip(clip: &Clip, t: f32, target: &AnimAsset, source: &AnimAsset, map: &[Option<usize>], same: bool, in_place: bool, locals: &mut [Trs]) {
    for c in &clip.channels {
        let Some(Some(ti)) = map.get(c.node) else { continue };
        let ti = *ti;
        let v = c.sample(t);
        let src = &source.skeleton.nodes[c.node];
        match c.path {
            ChannelPath::Rotation => locals[ti].r = Quat::from_vec4(v).normalize(),
            ChannelPath::Translation => {
                let mut p = v.truncate();
                if in_place && src.name.eq_ignore_ascii_case("root") {
                    p.x = src.rest.t.x;
                    p.z = src.rest.t.z;
                }
                if same {
                    locals[ti].t = p;
                } else if is_hips(&src.name) || src.name.eq_ignore_ascii_case("root") {
                    locals[ti].t = target.skeleton.nodes[ti].rest.t + (p - src.rest.t);
                }
            }
            ChannelPath::Scale => {
                if same {
                    locals[ti].s = v.truncate();
                }
            }
        }
    }
}

/// Avanza un tiempo de clip (bucle o hasta el final).
pub fn advance_time(t: f32, dt: f32, duration: f32, looping: bool) -> f32 {
    let t = t + dt;
    if duration <= 0.0 {
        0.0
    } else if looping {
        t.rem_euclid(duration)
    } else {
        t.clamp(0.0, duration)
    }
}

struct Job {
    animator: EntityId,
    source: Arc<AnimAsset>,
    clip: usize,
    time: f32,
    from: Option<(usize, f32)>,
    /// Peso del clip actual (1 = sin fundido).
    weight: f32,
    in_place: bool,
}

/// Estado del sistema (cachés y memoria reutilizada entre fotogramas).
#[derive(Default)]
pub struct Animation {
    /// Mapas fuente → destino por pareja de recursos. Guarda los `Arc` para
    /// que sus direcciones (la clave) no se reutilicen mientras exista la entrada.
    maps: HashMap<(usize, usize), (Arc<AnimAsset>, Arc<AnimAsset>, Arc<Vec<Option<usize>>>)>,
    warned: HashSet<String>,
    jobs: Vec<Job>,
    locals: Vec<Trs>,
    locals_from: Vec<Trs>,
    globals: Vec<Mat4>,
    palette: Vec<Mat4>,
    targets: Vec<(EntityId, EntityId, PathBuf)>,
}

impl Animation {
    fn warn_once(&mut self, key: String, msg: String) {
        if self.warned.insert(key) {
            log::warn("animación", msg);
        }
    }

    fn map(&mut self, target: &Arc<AnimAsset>, source: &Arc<AnimAsset>) -> Arc<Vec<Option<usize>>> {
        let key = (Arc::as_ptr(target) as usize, Arc::as_ptr(source) as usize);
        self.maps.entry(key).or_insert_with(|| (target.clone(), source.clone(), Arc::new(retarget_map(target, source)))).2.clone()
    }

    /// Un paso: avanza `dt` los Animator para los que `advance` es cierto y
    /// recalcula las paletas de todas las mallas con piel animadas.
    pub fn update(&mut self, scene: &mut Scene, root: Option<&Path>, dt: f32, advance: impl Fn(EntityId) -> bool) {
        self.jobs.clear();
        self.targets.clear();
        let animated: Vec<(EntityId, hecs::Entity)> = scene.world.query::<(&EntityId, &Animator)>().iter().map(|(id, _)| *id).filter_map(|id| scene.entity(id).map(|e| (id, e))).collect();
        if animated.is_empty() {
            return;
        }
        // 1. Avanzar y preparar el muestreo de cada Animator.
        for &(id, e) in &animated {
            if !scene.is_active_in_hierarchy(id) {
                continue;
            }
            let own = match scene.world.get::<&MeshRenderer>(e).ok().map(|m| m.mesh.clone()) {
                Some(MeshSource::Model(p)) => Some(p),
                _ => None,
            };
            let Ok(mut an) = scene.world.get::<&mut Animator>(e) else { continue };
            let src_path = if an.source.is_empty() { own.clone() } else { Some(an.source.clone()) };
            let Some(src_path) = src_path else { continue };
            let Some(source) = asset(&biblioteca::resolve(&src_path, root)) else { continue };
            if an.clip.is_empty() {
                if let Some(c) = source.clips.first() {
                    an.clip = c.name.clone();
                } else {
                    continue;
                }
            }
            if an.state.current != an.clip {
                an.state.current = an.clip.clone();
                an.state.time = 0.0;
                an.state.from = None;
                an.state.fade_left = 0.0;
            }
            let Some(ci) = source.clip(&an.clip) else {
                let (clip, path) = (an.clip.clone(), src_path.clone());
                drop(an);
                self.warn_once(format!("{clip}@{path}"), format!("no existe el clip '{clip}' en {path}"));
                continue;
            };
            let dur = source.clips[ci].duration;
            let step = if an.playing && advance(id) { dt * an.speed } else { 0.0 };
            an.state.time = advance_time(an.state.time, step, dur, an.looping);
            let mut from = None;
            if let Some((name, t)) = an.state.from.clone() {
                an.state.fade_left -= step.abs();
                match source.clip(&name) {
                    Some(fi) if an.state.fade_left > 0.0 => {
                        let ft = advance_time(t, step, source.clips[fi].duration, an.looping);
                        an.state.from = Some((name, ft));
                        from = Some((fi, ft));
                    }
                    _ => an.state.from = None,
                }
            }
            let weight = if from.is_some() { 1.0 - an.state.fade_left / an.state.fade.max(1e-4) } else { 1.0 };
            self.jobs.push(Job { animator: id, source, clip: ci, time: an.state.time, from, weight: weight.clamp(0.0, 1.0), in_place: an.in_place });
        }
        if self.jobs.is_empty() {
            return;
        }
        // 2. Mallas con piel: la del Animator y las de sus hijos sin Animator propio.
        let hier = scene.hierarchy();
        for job in &self.jobs {
            let mut stack = vec![job.animator];
            while let Some(id) = stack.pop() {
                let Some(e) = scene.entity(id) else { continue };
                if id != job.animator && scene.world.satisfies::<&Animator>(e) {
                    continue;
                }
                if let Ok(mr) = scene.world.get::<&MeshRenderer>(e)
                    && let MeshSource::Model(p) = &mr.mesh
                {
                    self.targets.push((job.animator, id, biblioteca::resolve(p, root)));
                }
                stack.extend_from_slice(hier.children_of(id));
            }
        }
        // 3. Pose y paleta de cada malla.
        let mut inserts: Vec<(hecs::Entity, SkinPose)> = Vec::new();
        let targets = std::mem::take(&mut self.targets);
        for (animator, mesh_id, path) in &targets {
            let Some(target) = asset(path) else { continue };
            if !target.has_skin() {
                continue;
            }
            let Some(ji) = self.jobs.iter().position(|j| j.animator == *animator) else { continue };
            let source = self.jobs[ji].source.clone();
            let map = self.map(&target, &source);
            let same = Arc::ptr_eq(&target, &source);
            let job = &self.jobs[ji];
            self.locals.clear();
            self.locals.extend(target.skeleton.nodes.iter().map(|n| n.rest));
            apply_clip(&source.clips[job.clip], job.time, &target, &source, &map, same, job.in_place, &mut self.locals);
            if let Some((fi, ft)) = job.from {
                self.locals_from.clear();
                self.locals_from.extend(target.skeleton.nodes.iter().map(|n| n.rest));
                apply_clip(&source.clips[fi], ft, &target, &source, &map, same, job.in_place, &mut self.locals_from);
                for (a, b) in self.locals.iter_mut().zip(&self.locals_from) {
                    *a = b.lerp(a, job.weight);
                }
            }
            target.skeleton.globals(&self.locals, &mut self.globals);
            target.skeleton.palette(&self.globals, &mut self.palette);
            let Some(e) = scene.entity(*mesh_id) else { continue };
            match scene.world.get::<&mut SkinPose>(e) {
                Ok(mut sp) => {
                    sp.palette.clear();
                    sp.palette.extend_from_slice(&self.palette);
                }
                Err(_) => inserts.push((e, SkinPose { palette: self.palette.clone() })),
            }
        }
        self.targets = targets;
        for (e, sp) in inserts {
            let _ = scene.world.insert_one(e, sp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Channel, Interpolation, Node, Skeleton};
    use glam::{Vec3, Vec4};

    fn rig(names: &[&str], hips_height: f32) -> AnimAsset {
        let mut sk = Skeleton::default();
        for (i, n) in names.iter().enumerate() {
            let t = if is_hips(n) { Vec3::new(0.0, hips_height, 0.0) } else if i == 0 { Vec3::ZERO } else { Vec3::Y * 0.5 };
            sk.nodes.push(Node { name: n.to_string(), parent: if i == 0 { None } else { Some(i - 1) }, rest: Trs { t, ..Default::default() } });
        }
        sk.joints = (0..names.len()).collect();
        let mut g = Vec::new();
        sk.globals(&sk.rest_locals(), &mut g);
        sk.inverse_bind = g.iter().map(|m| m.inverse()).collect();
        AnimAsset { skeleton: sk, clips: vec![] }
    }

    #[test]
    fn retargets_by_bone_name_with_hips_delta_and_in_place_root() {
        // Fuente: root → pelvis (1.0 m) → spine. Destino: otro orden y cadera a 1.2 m.
        let mut src = rig(&["root", "pelvis", "spine_01"], 1.0);
        let dst = rig(&["root", "pelvis", "spine_01", "extra"], 1.2);
        let rot = Quat::from_rotation_x(0.5);
        src.clips.push(Clip {
            name: "Walk".into(),
            duration: 1.0,
            channels: vec![
                Channel { node: 0, path: ChannelPath::Translation, interpolation: Interpolation::Linear, times: vec![0.0, 1.0], values: vec![Vec4::ZERO, Vec4::new(0.0, 0.0, 3.0, 0.0)] },
                Channel { node: 1, path: ChannelPath::Translation, interpolation: Interpolation::Linear, times: vec![0.0, 1.0], values: vec![Vec4::new(0.0, 1.0, 0.0, 0.0), Vec4::new(0.0, 0.9, 0.0, 0.0)] },
                Channel { node: 2, path: ChannelPath::Rotation, interpolation: Interpolation::Linear, times: vec![0.0], values: vec![Vec4::from(rot)] },
                Channel { node: 2, path: ChannelPath::Translation, interpolation: Interpolation::Linear, times: vec![0.0], values: vec![Vec4::new(0.0, 9.0, 0.0, 0.0)] },
            ],
        });
        let map = retarget_map(&dst, &src);
        assert_eq!(map, vec![Some(0), Some(1), Some(2)]);
        let mut locals = dst.skeleton.rest_locals();
        apply_clip(&src.clips[0], 1.0, &dst, &src, &map, false, true, &mut locals);
        // root quieto en horizontal (in_place).
        assert!(locals[0].t.length() < 1e-6, "{:?}", locals[0].t);
        // Cadera: reposo del destino + diferencia de la fuente (1.2 - 0.1).
        assert!((locals[1].t.y - 1.1).abs() < 1e-5, "{:?}", locals[1].t);
        // Columna: rotación copiada, traslación del destino intacta.
        assert!(locals[2].r.dot(rot).abs() > 0.9999);
        assert_eq!(locals[2].t, Vec3::Y * 0.5);
        // Sin in_place el root avanza.
        let mut l2 = dst.skeleton.rest_locals();
        apply_clip(&src.clips[0], 1.0, &dst, &src, &map, false, false, &mut l2);
        assert!((l2[0].t.z - 3.0).abs() < 1e-5);
    }

    #[test]
    fn time_advances_with_loop_and_clamp() {
        assert!((advance_time(0.9, 0.2, 1.0, true) - 0.1).abs() < 1e-5);
        assert_eq!(advance_time(0.9, 0.2, 1.0, false), 1.0);
        assert!((advance_time(0.1, -0.2, 1.0, true) - 0.9).abs() < 1e-5, "velocidad negativa");
        assert_eq!(advance_time(3.0, 1.0, 0.0, true), 0.0);
    }

    #[test]
    fn system_writes_skin_pose_for_animator_and_children() {
        // Un modelo con piel en disco: el test de glTF mínimo no tiene piel, así
        // que se registra el recurso en la caché con rutas ficticias.
        let mut a = rig(&["root", "pelvis"], 1.0);
        a.clips.push(Clip {
            name: "Up".into(),
            duration: 2.0,
            channels: vec![Channel { node: 1, path: ChannelPath::Translation, interpolation: Interpolation::Linear, times: vec![0.0, 2.0], values: vec![Vec4::new(0.0, 1.0, 0.0, 0.0), Vec4::new(0.0, 3.0, 0.0, 0.0)] }],
        });
        let a = Arc::new(a);
        let dir = std::env::temp_dir().join(format!("maker-anim-{}", std::process::id()));
        cache_asset(&dir.join("body.glb"), a.clone());
        cache_asset(&dir.join("hat.glb"), Arc::new(rig(&["root", "pelvis"], 1.0)));
        let mut s = Scene::new();
        let body = s.spawn("Cuerpo");
        s.insert(body, MeshRenderer { mesh: MeshSource::Model("body.glb".into()), material: Material::default(), cast_shadows: true });
        s.insert(body, Animator::new("", "Up"));
        let hat = s.spawn("Sombrero");
        s.insert(hat, MeshRenderer { mesh: MeshSource::Model("hat.glb".into()), material: Material::default(), cast_shadows: true });
        s.set_parent(hat, Some(body), false);
        let mut sys = Animation::default();
        sys.update(&mut s, Some(&dir), 1.0, |_| true);
        let pose = s.get::<SkinPose>(body).unwrap();
        // A t=1 la cadera sube 1 m: la paleta del hueso 1 es una traslación de +1 en Y.
        assert!((pose.palette[1].w_axis.y - 1.0).abs() < 1e-5, "{:?}", pose.palette[1]);
        let hat_pose = s.get::<SkinPose>(hat).unwrap();
        assert_eq!(hat_pose.palette.len(), 2);
        assert!((hat_pose.palette[1].w_axis.y - 1.0).abs() < 1e-5, "el hijo sigue al Animator del padre");
        // Sin avanzar (editor sin seleccionar): la pose no cambia.
        sys.update(&mut s, Some(&dir), 1.0, |_| false);
        assert!((s.get::<SkinPose>(body).unwrap().palette[1].w_axis.y - 1.0).abs() < 1e-5);
    }
}
