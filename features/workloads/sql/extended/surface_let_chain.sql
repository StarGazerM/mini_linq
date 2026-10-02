-- The second arithmetic binding depends on the first.
WITH sums AS (SELECT c0 + c1 AS total FROM "mini-linq/raw/0"),
shifted AS (SELECT total, total * 2 - 1 AS shifted FROM sums)
SELECT DISTINCT total, shifted FROM shifted WHERE shifted > 0 ORDER BY 1, 2;
