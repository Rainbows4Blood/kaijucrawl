-- Independent D&D classification, multi-edition memberships and summary invalidation.
INSERT INTO analysis_groups VALUES
 ('dnd','root','D&D compatibility',NULL),
 ('dnd','dnd_only','D&D','root'),
 ('dnd','dnd_and_others','D&D and others','root'),
 ('dnd','no','No','root'),
 ('dnd_editions','root','D&D editions',NULL),
 ('dnd_editions','identified','Editions identified','root'),
 ('dnd_editions','unknown','No edition identified','root'),
 ('dnd_editions','0e','0e','identified'),
 ('dnd_editions','bx','B/X','identified'),
 ('dnd_editions','adnd_1e','AD&D 1e','identified'),
 ('dnd_editions','adnd_2e','AD&D 2e','identified'),
 ('dnd_editions','3e','3e','identified'),
 ('dnd_editions','3_5e','3.5e','identified'),
 ('dnd_editions','4e','4e','identified'),
 ('dnd_editions','5e_2014','5e 2014','identified'),
 ('dnd_editions','5e_2024','5e 2024','identified');
DROP TRIGGER classification_inputs_changed;

CREATE TRIGGER classification_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system IN ('book_role','solo_play','dnd','dnd_editions');
END;

CREATE TRIGGER dnd_eligibility_removed AFTER DELETE ON book_classifications WHEN old.system='dnd' BEGIN
 DELETE FROM book_classifications WHERE target=old.target AND product_id=old.product_id AND system='dnd_editions';
END;
CREATE TRIGGER dnd_eligibility_changed AFTER UPDATE OF category,algorithm ON book_classifications
WHEN new.system='dnd' AND (new.category NOT IN ('dnd_only','dnd_and_others') OR new.algorithm!=old.algorithm) BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system='dnd_editions';
END;
CREATE TRIGGER analysis_capture_added AFTER INSERT ON products BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
CREATE TRIGGER analysis_capture_removed AFTER DELETE ON products BEGIN
 UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1;
END;
