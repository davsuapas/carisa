1. Sistema de log integral. Sobre todo la proyección, stream es importante cuando el orden de mensajes es incorrecto, no hay id de asistente para una tools, etc.
2. Configurar el scope de struct, módulos, funciones, etc a lo mínimo posible.
3. Resolver step y turn después de crear el workflow. Habrá que mantenerlo en temporal y seguramente habrá que cambiar los campos. No tiene sentido que turn_id sea un MessageId.
4. En la instrucciones agentes que no se expongan los errores internos hacia fuera en las apis públicas.
