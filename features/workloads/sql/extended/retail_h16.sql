WITH
  Part AS (SELECT DISTINCT c0 AS part_key FROM "mini-linq/raw/0"),
  Partsupp AS (SELECT DISTINCT c0 AS part_key, c1 AS suppkey FROM "mini-linq/raw/1"),
  BadSupplier AS (SELECT DISTINCT c0 AS suppkey FROM "mini-linq/raw/2")
SELECT DISTINCT p.part_key, ps.suppkey FROM Part p JOIN Partsupp ps ON ps.part_key = p.part_key WHERE NOT EXISTS (SELECT 1 FROM BadSupplier s WHERE s.suppkey = ps.suppkey)
ORDER BY 1, 2;
