WITH
  Seed AS (SELECT DISTINCT c0 FROM "mini-linq/raw/0"),
  Edge AS (SELECT DISTINCT c0,c1 FROM "mini-linq/raw/1")
SELECT DISTINCT * FROM (
WITH RECURSIVE Reach(x) AS (
  SELECT Edge.c1 FROM Seed JOIN Edge ON Seed.c0=Edge.c0
  UNION
  SELECT Edge.c1 FROM Reach JOIN Edge ON Reach.x=Edge.c0
) SELECT x FROM Reach
) AS result
ORDER BY 1;
