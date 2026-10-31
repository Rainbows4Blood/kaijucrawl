DROP TRIGGER IF EXISTS warhammer_scope_added;
DROP TRIGGER IF EXISTS warhammer_scope_changed;
DROP TRIGGER IF EXISTS warhammer_scope_removed;
DROP TRIGGER IF EXISTS warhammer_inputs_changed;
DROP TRIGGER IF EXISTS warhammer_context_removed;
DROP TRIGGER IF EXISTS warhammer_context_family_changed;
DROP TABLE IF EXISTS warhammer_context;
DROP TABLE IF EXISTS warhammer_scope;
DROP VIEW IF EXISTS warhammer_scope_source;
DELETE FROM book_classifications WHERE system IN ('warhammer_fantasy','warhammer_40k');
DELETE FROM analysis_groups WHERE system IN ('warhammer_fantasy','warhammer_40k');
DROP TRIGGER IF EXISTS osr_levels_removed;
DROP TRIGGER IF EXISTS osr_levels_prerequisite_removed;
DROP TRIGGER IF EXISTS osr_levels_prerequisite_changed;
DROP TRIGGER IF EXISTS osr_levels_membership_added;
DROP TRIGGER IF EXISTS osr_levels_membership_removed;
DROP TRIGGER IF EXISTS osr_levels_membership_changed;
DROP VIEW IF EXISTS osr_level_source;
DROP VIEW IF EXISTS osr_levels_eligible;
DROP TABLE IF EXISTS osr_level_ranges;
DROP TABLE IF EXISTS osr_levels;
-- Disposable migration fixture: restore the pre-OSR schema only.
DROP TRIGGER IF EXISTS osr_scope_added;
DROP TRIGGER IF EXISTS osr_scope_changed;
DROP TRIGGER IF EXISTS osr_scope_removed;
DROP TRIGGER IF EXISTS osr_inputs_changed;
DELETE FROM book_classifications WHERE system='osr';
DELETE FROM analysis_groups WHERE system='osr' AND parent_key='identified';
DELETE FROM analysis_groups WHERE system='osr' AND parent_key='root';
DELETE FROM analysis_groups WHERE system='osr';
DROP TABLE IF EXISTS osr_scope;
DROP VIEW IF EXISTS osr_scope_source;
DROP TRIGGER IF EXISTS earth_editions_scope_removed;
DROP TRIGGER IF EXISTS earth_editions_inputs_changed;
DELETE FROM book_classifications WHERE system IN ('shadowrun_editions','coc_editions');
DELETE FROM analysis_groups WHERE system IN ('shadowrun_editions','coc_editions') AND parent_key='identified';
DELETE FROM analysis_groups WHERE system IN ('shadowrun_editions','coc_editions') AND parent_key='root';
DELETE FROM analysis_groups WHERE system IN ('shadowrun_editions','coc_editions');
DROP TRIGGER IF EXISTS earth_context_removed;
DROP TRIGGER IF EXISTS earth_context_inputs_changed;
DROP TRIGGER IF EXISTS earth_scope_changed;
DROP TRIGGER IF EXISTS earth_scope_added;
DROP TABLE IF EXISTS earth_context;
DROP TABLE IF EXISTS earth_scope;
DROP VIEW IF EXISTS earth_scope_source;
-- Disposable fixture only: remove schema 11 before an earlier migration test.
DROP TRIGGER dnd_context_removed;
DROP TRIGGER dnd_settings_eligibility_removed;
DROP TRIGGER dnd_context_role_changed;
DROP TABLE dnd_context;
DELETE FROM book_classifications WHERE system='dnd_settings';
DELETE FROM analysis_groups WHERE system='dnd_settings' AND key='al_qadim';
DELETE FROM analysis_groups WHERE system='dnd_settings' AND parent_key='identified';
DELETE FROM analysis_groups WHERE system='dnd_settings' AND parent_key='root';
DELETE FROM analysis_groups WHERE system='dnd_settings';
-- Disposable fixture only: remove schema 9 additions before older migration tests.
DROP TRIGGER analysis_capture_added;
DROP TRIGGER analysis_capture_removed;
DROP TRIGGER dnd_eligibility_removed;
DROP TRIGGER dnd_eligibility_changed;
DELETE FROM analysis_groups WHERE system='dnd_editions' AND parent_key='identified';
DELETE FROM analysis_groups WHERE system IN ('dnd','dnd_editions') AND parent_key IS NOT NULL;
DELETE FROM analysis_groups WHERE system IN ('dnd','dnd_editions');
DROP TRIGGER classification_inputs_changed;

CREATE TRIGGER classification_inputs_changed AFTER UPDATE ON products
WHEN old.title IS NOT new.title OR old.description_text IS NOT new.description_text
 OR old.publisher IS NOT new.publisher OR old.page_count IS NOT new.page_count
 OR json_extract(old.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
 IS NOT json_extract(new.record,'$.authors','$.rule_systems','$.genres','$.themes','$.product_types','$.languages','$.formats','$.edition','$.categories','$.filters','$.primary_filters','$.is_bundle','$.flags.isAudio','$.flags.isVideo')
BEGIN
 DELETE FROM book_classifications WHERE target=new.target AND product_id=new.product_id AND system IN ('book_role','solo_play');
END;
