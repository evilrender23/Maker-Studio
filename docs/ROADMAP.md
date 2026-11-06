# Hoja de ruta

Estado de la versión 0.1 frente a lo que ofrecen Unity, Unreal y Godot.
✅ hecho · ⚠️ parcial · ❌ pendiente.

| Área | Estado | Notas |
|---|---|---|
| ECS + jerarquía de escena | ✅ | hecs, ids estables, reparentar conservando el mundo |
| Escenas en texto | ✅ | `.mscene` JSON |
| Prefabs / escenas anidadas | ❌ | siguiente gran paso (Unity prefabs, Godot escenas instanciables) |
| Selección múltiple | ❌ | hoy una entidad a la vez |
| Copiar/pegar entre escenas | ❌ | |
| Gizmos mover/rotar/escalar | ✅ | local/global, ajuste |
| Deshacer/rehacer | ✅ | instantáneas de escena |
| Play en editor + Pausa + Paso | ✅ | restauración al parar |
| Scripting en Rust con recarga en caliente | ✅ | C-ABI, pánicos atrapados |
| Conservar el estado de los scripts al recargar en Play | ❌ | hoy se recarga al parar |
| Eventos de colisión / disparadores para scripts | ❌ | `on_collision_enter`, `on_trigger_enter` |
| Consultas de componentes desde scripts | ⚠️ | transform, color, velocidad; falta acceso genérico |
| Física: cuerpos, formas, raycast | ✅ | Rapier |
| Física: juntas, controlador de personaje, mallas como colisionador | ❌ | |
| PBR + sol con sombras + luces puntuales | ✅ | |
| Texturas y materiales como asset | ⚠️ | glTF: color base, normal, metal/rugosidad, oclusión y emisión; falta el material como asset propio |
| Modo Texturizado (pintar PBR sobre assets) | ⚠️ | estudio y motor integrado; interfaz pendiente, ver [TEXTURIZADO.md](TEXTURIZADO.md) |
| Sombras de luces puntuales, cascadas (CSM) | ❌ | un mapa de 2048² centrado en la cámara |
| Post-proceso (bloom, SSAO, TAA) | ❌ | |
| Instancing por GPU | ❌ | hoy un draw call por objeto con uniform dinámico |
| Importación glTF | ⚠️ | geometría y jerarquía; faltan materiales, texturas y animación |
| Animación esquelética | ❌ | |
| Audio | ❌ | |
| UI de juego (HUD) | ❌ | |
| Navegación (navmesh) | ❌ | |
| Exportar juego | ✅ | ejecutable + datos + scripts |
| Hub de proyectos | ✅ | |
| Tema claro de Hermes | ✅ | |
