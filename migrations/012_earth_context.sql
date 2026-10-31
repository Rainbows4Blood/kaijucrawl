-- Narrow analysis eligibility index; no Search, model work or capture rewrite.
CREATE VIEW earth_scope_source AS
SELECT DISTINCT p.target,p.product_id,p.rowid AS document,k.system
FROM products p JOIN json_each(p.record,'$.rule_systems') r
CROSS JOIN (SELECT 'shadowrun' AS system UNION ALL SELECT 'coc') k
WHERE r.type='text' AND ((k.system='shadowrun' AND instr(lower(r.value),'shadowrun')>0)
 OR (k.system='coc' AND lower(trim(r.value))='call of cthulhu'));
CREATE TABLE earth_scope (
 target TEXT NOT NULL,product_id TEXT NOT NULL,system TEXT NOT NULL CHECK(system IN ('shadowrun','coc')),
 document INTEGER NOT NULL,
 PRIMARY KEY(target,product_id,system),
 FOREIGN KEY(target,product_id) REFERENCES products(target,product_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE UNIQUE INDEX earth_scope_keyset ON earth_scope(system,document);
INSERT INTO earth_scope SELECT target,product_id,system,document FROM earth_scope_source;
CREATE TABLE earth_context (
 target TEXT NOT NULL,product_id TEXT NOT NULL,system TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('location','date')),
 location TEXT,date_start TEXT,date_end TEXT,precision TEXT,basis TEXT,
 algorithm TEXT NOT NULL,model TEXT NOT NULL,input_hash TEXT NOT NULL,
 calculated_at TEXT NOT NULL,run_id INTEGER NOT NULL REFERENCES analysis_runs(id),
 PRIMARY KEY(target,product_id,system,kind),
 FOREIGN KEY(target,product_id,system) REFERENCES earth_scope(target,product_id,system) ON DELETE CASCADE,
 CHECK(date_start IS NULL OR (length(date_start)=10 AND date(date_start) IS date_start AND date_start>='0001-01-01')),
 CHECK(date_end IS NULL OR (length(date_end)=10 AND date(date_end) IS date_end AND date_end>='0001-01-01')),
 CHECK(date_start IS NULL OR date_end IS NULL OR date_start<=date_end),
 CHECK((kind='location' AND date_start IS NULL AND date_end IS NULL AND precision IS NULL AND basis IS NULL)
    OR (kind='date' AND location IS NULL AND precision IS NOT NULL AND basis IS NOT NULL
        AND precision IN ('day','month','year','decade','range','era','unknown') AND basis IN ('explicit','modern','dark_ages','unknown')))
) WITHOUT ROWID;
CREATE INDEX earth_context_algorithm ON earth_context(system,kind,algorithm,target,product_id);
CREATE TRIGGER earth_scope_added AFTER INSERT ON products BEGIN
 INSERT INTO earth_scope SELECT target,product_id,system,document FROM earth_scope_source WHERE target=new.target AND product_id=new.product_id;
END;
CREATE TRIGGER earth_scope_changed AFTER UPDATE OF record ON products
WHEN json_extract(old.record,'$.rule_systems') IS NOT json_extract(new.record,'$.rule_systems') BEGIN
 DELETE FROM earth_scope WHERE target=new.target AND product_id=new.product_id;
 INSERT INTO earth_scope SELECT target,product_id,system,document FROM earth_scope_source WHERE target=new.target AND product_id=new.product_id;
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER earth_context_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM earth_context WHERE target=new.target AND product_id=new.product_id;
END;
CREATE TRIGGER earth_context_removed AFTER DELETE ON earth_context BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
