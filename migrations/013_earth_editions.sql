-- Edition groups share existing fenced results/history; no prior results are rewritten.
INSERT INTO analysis_groups VALUES
 ('shadowrun_editions','root','Shadowrun editions',NULL),
 ('shadowrun_editions','identified','Editions identified','root'),
 ('shadowrun_editions','unknown','Unknown','root'),
 ('shadowrun_editions','1e','1st edition','identified'),
 ('shadowrun_editions','2e','2nd edition','identified'),
 ('shadowrun_editions','3e','3rd edition','identified'),
 ('shadowrun_editions','4e','4th edition','identified'),
 ('shadowrun_editions','5e','5th edition','identified'),
 ('shadowrun_editions','6e','6th edition','identified'),
 ('shadowrun_editions','anarchy','Anarchy','identified'),
 ('shadowrun_editions','anarchy_2','Anarchy 2.0','identified'),
 ('coc_editions','root','Call of Cthulhu editions',NULL),
 ('coc_editions','identified','Editions identified','root'),
 ('coc_editions','unknown','Unknown','root'),
 ('coc_editions','1e','1st edition','identified'),
 ('coc_editions','2e','2nd edition','identified'),
 ('coc_editions','3e','3rd edition','identified'),
 ('coc_editions','4e','4th edition','identified'),
 ('coc_editions','5e','5th edition','identified'),
 ('coc_editions','6e','6th edition','identified'),
 ('coc_editions','7e','7th edition','identified');
CREATE TRIGGER earth_editions_scope_removed AFTER DELETE ON earth_scope BEGIN
 DELETE FROM book_classifications WHERE target=old.target AND product_id=old.product_id
 AND system=CASE old.system WHEN 'shadowrun' THEN 'shadowrun_editions' ELSE 'coc_editions' END;
END;
CREATE TRIGGER earth_editions_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system IN ('shadowrun_editions','coc_editions');
END;
