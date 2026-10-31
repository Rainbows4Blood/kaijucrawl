-- Independent solo-play groups. Existing captures, analyses and memberships are preserved.
INSERT INTO analysis_groups VALUES
 ('solo_play','root','Solo play',NULL),
 ('solo_play','solo_adventure','Solo Adventure','root'),
 ('solo_play','gamebook','Gamebook','root'),
 ('solo_play','neither','Neither','root');
DROP TRIGGER classification_inputs_changed;
CREATE TRIGGER classification_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system IN ('book_role','solo_play');
END;
