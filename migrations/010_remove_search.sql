-- Retire derived Search data only. Products, archives, analysis results and history stay intact.
-- Old search document IDs were products.rowid; analyses now keyset directly on that rowid.
DROP TRIGGER IF EXISTS search_product_insert;
DROP TRIGGER IF EXISTS search_product_update;
DROP TRIGGER IF EXISTS search_product_delete;
DROP TRIGGER IF EXISTS search_document_insert;
DROP TRIGGER IF EXISTS search_document_delete;
DROP VIEW IF EXISTS search_source;
DROP TABLE IF EXISTS search_facets;
DROP TABLE IF EXISTS search_fts;
DROP TABLE IF EXISTS search_documents;
DROP TABLE IF EXISTS search_progress;
