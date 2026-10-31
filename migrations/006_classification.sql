-- Analysis-owned groups and provenance. No capture rewrite or automatic API work.
ALTER TABLE analysis_runs ADD COLUMN classification TEXT;
CREATE TABLE analysis_groups (
 system TEXT NOT NULL, key TEXT NOT NULL, label TEXT NOT NULL, parent_key TEXT,
 PRIMARY KEY(system,key), FOREIGN KEY(system,parent_key) REFERENCES analysis_groups(system,key)
) WITHOUT ROWID;
INSERT INTO analysis_groups VALUES
 ('book_role','root','Book role',NULL),
 ('book_role','core_rulebook','Core Rulebook','root'),
 ('book_role','supplement','Supplement','root'),
 ('book_role','adventure','Adventure','root'),
 ('book_role','none','None','root');
CREATE TABLE book_classifications (
 target TEXT NOT NULL, product_id TEXT NOT NULL, system TEXT NOT NULL,
 category TEXT NOT NULL, algorithm TEXT NOT NULL, model TEXT NOT NULL,
 confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1), probabilities TEXT NOT NULL,
 input_hash TEXT NOT NULL, calculated_at TEXT NOT NULL, run_id INTEGER NOT NULL REFERENCES analysis_runs(id),
 PRIMARY KEY(target,product_id,system),
 FOREIGN KEY(target,product_id) REFERENCES products(target,product_id) ON DELETE CASCADE,
 FOREIGN KEY(system,category) REFERENCES analysis_groups(system,key)
) WITHOUT ROWID;
CREATE INDEX classification_category ON book_classifications(system,algorithm,category,target,product_id);
CREATE TABLE book_group_memberships (
 target TEXT NOT NULL, product_id TEXT NOT NULL, system TEXT NOT NULL, group_key TEXT NOT NULL,
 PRIMARY KEY(target,product_id,system,group_key),
 FOREIGN KEY(target,product_id,system) REFERENCES book_classifications(target,product_id,system) ON DELETE CASCADE,
 FOREIGN KEY(system,group_key) REFERENCES analysis_groups(system,key)
) WITHOUT ROWID;
CREATE INDEX group_members ON book_group_memberships(system,group_key,target,product_id);
CREATE TABLE classification_attempts (
 run_id INTEGER NOT NULL REFERENCES analysis_runs(id), document INTEGER NOT NULL,
 target TEXT NOT NULL, product_id TEXT NOT NULL, input_hash TEXT,
 state TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '',
 PRIMARY KEY(run_id,document)
) WITHOUT ROWID;
CREATE TRIGGER classification_removed AFTER DELETE ON book_classifications BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
CREATE VIEW classification_source AS SELECT p.target,p.product_id,
 json_object('title',p.title,'description',p.description_text,'publisher',p.publisher,'page_count',p.page_count,
 'authors',json_extract(p.record,'$.authors'),'rule_systems',json_extract(p.record,'$.rule_systems'),
 'genres',json_extract(p.record,'$.genres'),'themes',json_extract(p.record,'$.themes'),
 'source_product_types',json_extract(p.record,'$.product_types'),'languages',json_extract(p.record,'$.languages'),
 'formats',json_extract(p.record,'$.formats'),'edition',json_extract(p.record,'$.edition'),
 'source_categories',json((SELECT json_group_array(json_extract(value,'$.name')) FROM json_each(p.record,'$.categories'))),
 'source_tags',json((SELECT json_group_array(json_extract(value,'$.name')) FROM json_each(p.record,'$.filters'))),
 'source_primary_tags',json((SELECT json_group_array(json_extract(value,'$.name')) FROM json_each(p.record,'$.primary_filters'))),
 'is_bundle',json_extract(p.record,'$.is_bundle'),'is_audio',json_extract(p.record,'$.flags.isAudio'),
 'is_video',json_extract(p.record,'$.flags.isVideo')) AS evidence FROM products p;
CREATE TRIGGER classification_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system='book_role';
END;
