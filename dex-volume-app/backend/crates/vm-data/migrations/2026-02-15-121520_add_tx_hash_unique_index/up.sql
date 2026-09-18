-- Collect duplicate IDs into a temp table (single pass with window function)
CREATE TEMP TABLE dup_ids AS
SELECT id FROM (
    SELECT id, ROW_NUMBER() OVER (PARTITION BY tx_hash ORDER BY id) AS rn
    FROM transactions
) t WHERE rn > 1;

-- Index the temp table for fast deletes
CREATE INDEX ON dup_ids (id);

-- Delete duplicates by primary key (fast indexed lookup)
DELETE FROM transactions WHERE id IN (SELECT id FROM dup_ids);

DROP TABLE dup_ids;
