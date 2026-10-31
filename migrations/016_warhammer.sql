-- Additive Rule System routing and separate enrichment; no capture rewrites.
CREATE VIEW warhammer_scope_source AS
SELECT DISTINCT p.target,p.product_id,p.rowid AS document,k.system
FROM products p JOIN json_each(p.record,'$.rule_systems') r
CROSS JOIN (SELECT 'warhammer_fantasy' AS system UNION ALL SELECT 'warhammer_40k') k
WHERE r.type='text' AND (
 lower(trim(r.value))='warhammer'
 OR (k.system='warhammer_fantasy' AND (
 lower(trim(r.value)) IN ('wfrp','warhammer fantasy','warhammer fantasy roleplay','warhammer fantasy roleplaying game','warhammer: the old world roleplaying game','warhammer the old world roleplaying game','warhammer: the old world','warhammer the old world','soulbound','age of sigmar: soulbound','warhammer age of sigmar: soulbound','warhammer: age of sigmar: soulbound')
 OR lower(trim(r.value)) LIKE 'warhammer fantasy roleplay %' OR lower(trim(r.value)) LIKE 'wfrp %'))
 OR (k.system='warhammer_40k' AND (
 lower(trim(r.value)) IN ('warhammer 40k','warhammer 40k roleplay','warhammer 40,000','warhammer 40,000 roleplay','warhammer 40000','warhammer 40000 roleplay','dark heresy','dark heresy 2nd edition','dark heresy second edition','rogue trader','only war','deathwatch','black crusade','imperium maledictum','wrath and glory','wrath & glory','warhammer 40,000: wrath & glory','warhammer 40,000: imperium maledictum')
 OR lower(trim(r.value)) LIKE 'dark heresy %')));
CREATE TABLE warhammer_scope (
 target TEXT NOT NULL,product_id TEXT NOT NULL,system TEXT NOT NULL CHECK(system IN ('warhammer_fantasy','warhammer_40k')),
 document INTEGER NOT NULL,PRIMARY KEY(target,product_id,system),
 FOREIGN KEY(target,product_id) REFERENCES products(target,product_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE UNIQUE INDEX warhammer_scope_keyset ON warhammer_scope(system,document);
INSERT INTO warhammer_scope SELECT target,product_id,system,document FROM warhammer_scope_source;
INSERT INTO analysis_groups VALUES
 ('warhammer_fantasy','root','Warhammer Fantasy',NULL),
 ('warhammer_fantasy','identified','Games identified','root'),
 ('warhammer_fantasy','unknown','Family confirmed; game unknown','root'),
 ('warhammer_fantasy','uncertain','Family uncertain','root'),
 ('warhammer_fantasy','not_applicable','Not this RPG family','root'),
 ('warhammer_fantasy','wfrp_1e','WFRP 1e','identified'),
 ('warhammer_fantasy','wfrp_2e','WFRP 2e','identified'),
 ('warhammer_fantasy','wfrp_3e','WFRP 3e','identified'),
 ('warhammer_fantasy','wfrp_4e','WFRP 4e','identified'),
 ('warhammer_fantasy','wfrp_5e','WFRP 5e','identified'),
 ('warhammer_fantasy','old_world','Warhammer: The Old World Roleplaying Game','identified'),
 ('warhammer_fantasy','soulbound','Warhammer Age of Sigmar: Soulbound','identified');
INSERT INTO analysis_groups VALUES
 ('warhammer_40k','root','Warhammer 40k',NULL),
 ('warhammer_40k','identified','Games identified','root'),
 ('warhammer_40k','unknown','Family confirmed; game unknown','root'),
 ('warhammer_40k','uncertain','Family uncertain','root'),
 ('warhammer_40k','not_applicable','Not this RPG family','root'),
 ('warhammer_40k','dark_heresy','Dark Heresy','identified'),
 ('warhammer_40k','dark_heresy_2e','Dark Heresy 2nd Edition','identified'),
 ('warhammer_40k','rogue_trader','Rogue Trader','identified'),
 ('warhammer_40k','only_war','Only War','identified'),
 ('warhammer_40k','deathwatch','Deathwatch','identified'),
 ('warhammer_40k','black_crusade','Black Crusade','identified'),
 ('warhammer_40k','imperium_maledictum','Imperium Maledictum','identified'),
 ('warhammer_40k','wrath_glory','Wrath and Glory','identified');
CREATE TABLE warhammer_context (
 target TEXT NOT NULL,product_id TEXT NOT NULL,system TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('locations','date')),
 locations TEXT CHECK(locations IS NULL OR (json_valid(locations) AND json_type(locations)='array' AND json_array_length(locations)<=64)),
 in_universe_date TEXT CHECK(in_universe_date IS NULL OR length(in_universe_date) BETWEEN 1 AND 1024),
 algorithm TEXT NOT NULL,model TEXT NOT NULL,input_hash TEXT NOT NULL,
 calculated_at TEXT NOT NULL,run_id INTEGER NOT NULL REFERENCES analysis_runs(id),
 PRIMARY KEY(target,product_id,system,kind),
 FOREIGN KEY(target,product_id,system) REFERENCES book_classifications(target,product_id,system) ON DELETE CASCADE,
 FOREIGN KEY(target,product_id,system) REFERENCES warhammer_scope(target,product_id,system) ON DELETE CASCADE,
 CHECK((kind='locations' AND locations IS NOT NULL AND in_universe_date IS NULL) OR (kind='date' AND locations IS NULL))
) WITHOUT ROWID;
CREATE INDEX warhammer_context_algorithm ON warhammer_context(system,kind,algorithm,target,product_id);
CREATE TRIGGER warhammer_scope_added AFTER INSERT ON products BEGIN
 INSERT INTO warhammer_scope SELECT target,product_id,system,document FROM warhammer_scope_source WHERE target=new.target AND product_id=new.product_id;
END;
CREATE TRIGGER warhammer_scope_changed AFTER UPDATE OF record ON products
WHEN json_extract(old.record,'$.rule_systems') IS NOT json_extract(new.record,'$.rule_systems') BEGIN
 DELETE FROM warhammer_scope WHERE target=new.target AND product_id=new.product_id;
 INSERT INTO warhammer_scope SELECT target,product_id,system,document FROM warhammer_scope_source WHERE target=new.target AND product_id=new.product_id;
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER warhammer_scope_removed AFTER DELETE ON warhammer_scope BEGIN
 DELETE FROM book_classifications WHERE target=old.target AND product_id=old.product_id AND system=old.system;
END;
CREATE TRIGGER warhammer_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system IN ('warhammer_fantasy','warhammer_40k');
END;
CREATE TRIGGER warhammer_context_removed AFTER DELETE ON warhammer_context BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER warhammer_context_family_changed AFTER UPDATE OF category,algorithm,input_hash ON book_classifications
WHEN new.system IN ('warhammer_fantasy','warhammer_40k') AND (new.category NOT IN ('identified','unknown') OR new.algorithm!=old.algorithm OR new.input_hash!=old.input_hash) BEGIN
 DELETE FROM warhammer_context WHERE target=new.target AND product_id=new.product_id AND system=new.system;
END;
