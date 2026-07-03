-- #11 (rastrear lo procesado + evidencia): añade la columna `evidencia` a `acciones`. Nullable
-- (compat con las filas existentes, que no la tienen) — texto libre que el operador/peer adjunta
-- al cerrar/procesar una acción como prueba de que se hizo (ej. link a un commit, captura, resumen
-- del resultado). ADD COLUMN es seguro en SQLite: no reescribe filas existentes, se rellenan NULL.
ALTER TABLE acciones ADD COLUMN evidencia TEXT;
