-- Derived nullable player counts; canonical captures and existing analyses stay intact.
CREATE TABLE adventure_players (
 target TEXT NOT NULL, product_id TEXT NOT NULL,
 system TEXT NOT NULL DEFAULT 'book_role' CHECK(system='book_role'),
 min_players INTEGER CHECK(min_players BETWEEN 1 AND 2147483647),
 max_players INTEGER CHECK(max_players BETWEEN 1 AND 2147483647),
 algorithm TEXT NOT NULL, model TEXT NOT NULL, input_hash TEXT NOT NULL,
 calculated_at TEXT NOT NULL, run_id INTEGER NOT NULL REFERENCES analysis_runs(id),
 CHECK(min_players IS NULL OR max_players IS NULL OR min_players<=max_players),
 PRIMARY KEY(target,product_id),
 FOREIGN KEY(target,product_id,system) REFERENCES book_classifications(target,product_id,system) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX adventure_players_range ON adventure_players(algorithm,min_players,max_players,target,product_id);
CREATE VIEW player_source AS SELECT target,product_id,
 json_object('title',title,'description',description_text) AS evidence FROM products;
CREATE TRIGGER player_counts_removed AFTER DELETE ON adventure_players BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER player_role_changed AFTER UPDATE OF category,algorithm ON book_classifications
WHEN new.system='book_role' AND (new.category!='adventure' OR new.algorithm!=old.algorithm)
BEGIN
 DELETE FROM adventure_players WHERE target=new.target AND product_id=new.product_id;
END;
CREATE TRIGGER player_inputs_changed AFTER UPDATE OF title,description_text ON products
WHEN new.title IS NOT old.title OR new.description_text IS NOT old.description_text
BEGIN
 DELETE FROM adventure_players WHERE target=new.target AND product_id=new.product_id;
END;
