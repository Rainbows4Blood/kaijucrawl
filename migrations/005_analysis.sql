-- Explicit, persisted analysis. No score calculation or capture rewrite at startup.
CREATE TABLE IF NOT EXISTS analysis_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT, algorithm TEXT NOT NULL,
 state TEXT NOT NULL, scope TEXT NOT NULL, query TEXT,
 started_at TEXT NOT NULL, finished_at TEXT, detail TEXT NOT NULL DEFAULT '',
 total INTEGER NOT NULL, processed INTEGER NOT NULL DEFAULT 0,
 scored INTEGER NOT NULL DEFAULT 0, skipped INTEGER NOT NULL DEFAULT 0,
 last_id INTEGER NOT NULL DEFAULT 0, upper_id INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS one_running_analysis ON analysis_runs((1)) WHERE state='running';
CREATE TABLE IF NOT EXISTS analysis_revision(singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL);
INSERT INTO analysis_revision VALUES(1,0) ON CONFLICT DO NOTHING;
CREATE TABLE IF NOT EXISTS relevance_scores (
 target TEXT NOT NULL, product_id TEXT NOT NULL, algorithm TEXT NOT NULL,
 score INTEGER CHECK(score BETWEEN 0 AND 5000000),
 input_rating REAL, input_count INTEGER,
 calculated_at TEXT NOT NULL, run_id INTEGER NOT NULL REFERENCES analysis_runs(id),
 PRIMARY KEY(target,product_id),
 FOREIGN KEY(target,product_id) REFERENCES products(target,product_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS relevance_rank ON relevance_scores(algorithm,score DESC,target,product_id);
CREATE TRIGGER IF NOT EXISTS relevance_removed AFTER DELETE ON relevance_scores BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
-- Reprocessing unrelated metadata retains the score. Changed inputs invalidate it;
-- neither a trigger nor a search query computes replacement analysis.
CREATE TRIGGER IF NOT EXISTS relevance_inputs_changed AFTER UPDATE OF record ON products
WHEN json_extract(old.record,'$.rating') IS NOT json_extract(new.record,'$.rating')
  OR json_extract(old.record,'$.rating_count') IS NOT json_extract(new.record,'$.rating_count')
BEGIN
 DELETE FROM relevance_scores WHERE target=new.target AND product_id=new.product_id;
END;
