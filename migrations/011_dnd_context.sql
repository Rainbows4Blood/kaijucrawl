-- Separate D&D enrichment; existing captures/results/history are not rewritten.
INSERT INTO analysis_groups VALUES
 ('dnd_settings','root','D&D settings',NULL),
 ('dnd_settings','identified','Named settings','root'),
 ('dnd_settings','unknown','Unknown','root'),
 ('dnd_settings','any','Any Setting','root'),
 ('dnd_settings','forgotten_realms','Forgotten Realms — Toril','identified'),
 ('dnd_settings','greyhawk','Greyhawk — Oerth','identified'),
 ('dnd_settings','blackmoor','Blackmoor','identified'),
 ('dnd_settings','mystara','Mystara / The Known World','identified'),
 ('dnd_settings','dragonlance','Dragonlance — Krynn','identified'),
 ('dnd_settings','ravenloft','Ravenloft','identified'),
 ('dnd_settings','dark_sun','Dark Sun — Athas','identified'),
 ('dnd_settings','birthright','Birthright — Aebrynis','identified'),
 ('dnd_settings','planescape','Planescape','identified'),
 ('dnd_settings','spelljammer','Spelljammer','identified'),
 ('dnd_settings','eberron','Eberron','identified'),
 ('dnd_settings','nentir_vale','Nentir Vale / Points of Light','identified'),
 ('dnd_settings','exandria','Exandria','identified'),
 ('dnd_settings','al_qadim','Al-Qadim — Zakhara','forgotten_realms');

CREATE TABLE dnd_context (
 target TEXT NOT NULL,product_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('levels','locations','date')),
 system TEXT NOT NULL DEFAULT 'dnd' CHECK(system='dnd'),
 min_level INTEGER CHECK(min_level BETWEEN 0 AND 2147483647),
 max_level INTEGER CHECK(max_level BETWEEN 0 AND 2147483647),
 locations TEXT CHECK(locations IS NULL OR (json_valid(locations) AND json_type(locations)='array')),
 in_universe_date TEXT,
 algorithm TEXT NOT NULL,model TEXT NOT NULL,input_hash TEXT NOT NULL,
 calculated_at TEXT NOT NULL,run_id INTEGER NOT NULL REFERENCES analysis_runs(id),
 CHECK(min_level IS NULL OR max_level IS NULL OR min_level<=max_level),
 CHECK((kind='levels' AND locations IS NULL AND in_universe_date IS NULL)
    OR (kind='locations' AND locations IS NOT NULL AND min_level IS NULL AND max_level IS NULL AND in_universe_date IS NULL)
    OR (kind='date' AND min_level IS NULL AND max_level IS NULL AND locations IS NULL)),
 PRIMARY KEY(target,product_id,kind),
 FOREIGN KEY(target,product_id,system) REFERENCES book_classifications(target,product_id,system) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX dnd_context_algorithm ON dnd_context(kind,algorithm,target,product_id);
CREATE TRIGGER dnd_context_removed AFTER DELETE ON dnd_context BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
-- Existing source-evidence invalidation deletes the D&D prerequisite, cascading
-- context and triggering settings deletion; unrelated prices/ratings retain them.
CREATE TRIGGER dnd_settings_eligibility_removed AFTER DELETE ON book_classifications WHEN old.system='dnd' BEGIN
 DELETE FROM book_classifications WHERE target=old.target AND product_id=old.product_id AND system='dnd_settings';
END;
CREATE TRIGGER dnd_context_role_changed AFTER UPDATE OF category,algorithm ON book_classifications
WHEN new.system='dnd' AND (new.category NOT IN ('dnd_only','dnd_and_others') OR new.algorithm!=old.algorithm) BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system='dnd_settings';
 DELETE FROM dnd_context WHERE target=new.target AND product_id=new.product_id;
END;
