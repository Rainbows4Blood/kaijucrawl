-- OSR analysis eligibility only; preserve canonical records and existing results.
CREATE VIEW osr_scope_source AS
SELECT DISTINCT p.target,p.product_id,p.rowid AS document
FROM products p JOIN json_each(p.record,'$.rule_systems') r
WHERE r.type='text' AND lower(trim(r.value))='old-school revival (osr)';
CREATE TABLE osr_scope (
 target TEXT NOT NULL,product_id TEXT NOT NULL,document INTEGER NOT NULL,
 PRIMARY KEY(target,product_id),
 FOREIGN KEY(target,product_id) REFERENCES products(target,product_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE UNIQUE INDEX osr_scope_keyset ON osr_scope(document);
INSERT INTO osr_scope SELECT target,product_id,document FROM osr_scope_source;
INSERT INTO analysis_groups VALUES
 ('osr','root','OSR',NULL),
 ('osr','identified','OSR systems identified','root'),
 ('osr','not_osr','Not OSR','root'),
 ('osr','osric','OSRIC','identified'),
 ('osr','castles_crusades','Castles & Crusades','identified'),
 ('osr','labyrinth_lord','Labyrinth Lord','identified'),
 ('osr','dungeon_crawl_classics','Dungeon Crawl Classics','identified'),
 ('osr','swords_wizardry','Swords & Wizardry','identified'),
 ('osr','old_school_essentials','Old-School Essentials','identified'),
 ('osr','mork_borg','Mork Borg','identified'),
 ('osr','into_the_odd','Into the Odd','identified'),
 ('osr','shadowdark','Shadowdark RPG','identified'),
 ('osr','dragonslayer','Dragonslayer','identified'),
 ('osr','acks','Adventurer Conqueror King System (ACKS)','identified'),
 ('osr','dolmenwood','Dolmenwood','identified'),
 ('osr','cairn','Cairn','identified'),
 ('osr','vagabond','Vagabond','identified'),
 ('osr','other','Other OSR Games','identified');
CREATE TRIGGER osr_scope_added AFTER INSERT ON products BEGIN
 INSERT INTO osr_scope SELECT target,product_id,document FROM osr_scope_source WHERE target=new.target AND product_id=new.product_id;
END;
CREATE TRIGGER osr_scope_changed AFTER UPDATE OF record ON products
WHEN json_extract(old.record,'$.rule_systems') IS NOT json_extract(new.record,'$.rule_systems') BEGIN
 DELETE FROM osr_scope WHERE target=new.target AND product_id=new.product_id;
 INSERT INTO osr_scope SELECT target,product_id,document FROM osr_scope_source WHERE target=new.target AND product_id=new.product_id;
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER osr_scope_removed AFTER DELETE ON osr_scope BEGIN
 DELETE FROM book_classifications WHERE target=old.target AND product_id=old.product_id AND system='osr';
END;
CREATE TRIGGER osr_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system='osr';
END;
