-- Separate per-system adventure levels; existing source/results remain intact.
CREATE VIEW osr_levels_eligible AS
SELECT e.target,e.product_id,e.document FROM osr_scope e
CROSS JOIN book_classifications o ON o.target=e.target AND o.product_id=e.product_id AND o.system='osr' AND o.algorithm='osr-systems-1' AND o.category='identified'
CROSS JOIN book_classifications r ON r.target=e.target AND r.product_id=e.product_id AND r.system='book_role' AND r.algorithm='primary-book-role-1' AND r.category='adventure'
WHERE EXISTS(SELECT 1 FROM book_group_memberships m WHERE m.target=e.target AND m.product_id=e.product_id AND m.system='osr' AND m.group_key NOT IN ('root','identified','not_osr'));
CREATE VIEW osr_level_source AS
SELECT l.target,l.product_id,l.document,json_object('book',json(e.evidence),'requested_systems',json((
 SELECT json_group_array(group_key) FROM (SELECT group_key FROM book_group_memberships m WHERE m.target=l.target AND m.product_id=l.product_id AND m.system='osr' AND m.group_key NOT IN ('root','identified','not_osr') ORDER BY group_key)
))) AS evidence FROM osr_levels_eligible l JOIN classification_source e ON e.target=l.target AND e.product_id=l.product_id;
CREATE TABLE osr_levels (
 target TEXT NOT NULL,product_id TEXT NOT NULL,
 algorithm TEXT NOT NULL,model TEXT NOT NULL,input_hash TEXT NOT NULL,
 calculated_at TEXT NOT NULL,run_id INTEGER NOT NULL REFERENCES analysis_runs(id),
 known_count INTEGER NOT NULL CHECK(known_count>=0),unknown_count INTEGER NOT NULL CHECK(unknown_count>=0),not_applicable_count INTEGER NOT NULL CHECK(not_applicable_count>=0),
 CHECK(known_count+unknown_count+not_applicable_count BETWEEN 1 AND 64),
 PRIMARY KEY(target,product_id),
 FOREIGN KEY(target,product_id) REFERENCES products(target,product_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX osr_levels_algorithm ON osr_levels(algorithm,target,product_id);
CREATE TABLE osr_level_ranges (
 target TEXT NOT NULL,product_id TEXT NOT NULL,system TEXT NOT NULL,
 game TEXT NOT NULL DEFAULT '',status TEXT NOT NULL CHECK(status IN ('known','unknown','not_applicable')),
 min_level INTEGER CHECK(min_level BETWEEN 0 AND 2147483647),
 max_level INTEGER CHECK(max_level BETWEEN 0 AND 2147483647),
 CHECK(min_level IS NULL OR max_level IS NULL OR min_level<=max_level),
 CHECK((status='known' AND (min_level IS NOT NULL OR max_level IS NOT NULL)) OR (status!='known' AND min_level IS NULL AND max_level IS NULL)),
 CHECK(system IN ('osric','castles_crusades','labyrinth_lord','dungeon_crawl_classics','swords_wizardry','old_school_essentials','mork_borg','into_the_odd','shadowdark','dragonslayer','acks','dolmenwood','cairn','vagabond','other')),
 CHECK((system!='other' AND game='') OR (system='other' AND (game!='' OR status='unknown'))),
 PRIMARY KEY(target,product_id,system,game),
 FOREIGN KEY(target,product_id) REFERENCES osr_levels(target,product_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TRIGGER osr_levels_removed AFTER DELETE ON osr_levels BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
-- Canonical source changes already remove the OSR classification, cascading here.
CREATE TRIGGER osr_levels_prerequisite_removed AFTER DELETE ON book_classifications
WHEN old.system IN ('osr','book_role') BEGIN
 DELETE FROM osr_levels WHERE target=old.target AND product_id=old.product_id;
END;
CREATE TRIGGER osr_levels_prerequisite_changed AFTER UPDATE OF category,algorithm,input_hash ON book_classifications
WHEN new.system IN ('osr','book_role') AND (new.category IS NOT old.category OR new.algorithm IS NOT old.algorithm OR new.input_hash IS NOT old.input_hash) BEGIN
 DELETE FROM osr_levels WHERE target=new.target AND product_id=new.product_id;
END;
CREATE TRIGGER osr_levels_membership_added AFTER INSERT ON book_group_memberships WHEN new.system='osr' BEGIN
 DELETE FROM osr_levels WHERE target=new.target AND product_id=new.product_id;
END;
CREATE TRIGGER osr_levels_membership_removed AFTER DELETE ON book_group_memberships WHEN old.system='osr' BEGIN
 DELETE FROM osr_levels WHERE target=old.target AND product_id=old.product_id;
END;
CREATE TRIGGER osr_levels_membership_changed AFTER UPDATE ON book_group_memberships WHEN old.system='osr' OR new.system='osr' BEGIN
 DELETE FROM osr_levels WHERE (target=old.target AND product_id=old.product_id) OR (target=new.target AND product_id=new.product_id);
END;
