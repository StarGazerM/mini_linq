WITH
  Goal AS (SELECT DISTINCT c0 FROM "mini-linq/raw/0"),
  Edge AS (SELECT DISTINCT c0,c1 FROM "mini-linq/raw/1")
SELECT DISTINCT * FROM (
WITH RECURSIVE Reach(x) AS (
  SELECT Edge.c0 FROM Goal JOIN Edge ON Goal.c0=Edge.c1
  UNION
  SELECT Edge.c0 FROM Reach JOIN Edge ON Reach.x=Edge.c1
) SELECT x FROM Reach
) AS result
ORDER BY 1;
