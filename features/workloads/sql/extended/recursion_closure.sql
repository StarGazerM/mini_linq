WITH
  Edge AS (SELECT DISTINCT c0,c1 FROM "mini-linq/raw/0")
SELECT DISTINCT * FROM (
WITH RECURSIVE Reach(x,y) AS (
  SELECT c0,c1 FROM Edge WHERE c0>=0
  UNION
  SELECT Reach.x,Edge.c1 FROM Reach JOIN Edge ON Reach.y=Edge.c0
) SELECT x,y FROM Reach
) AS result
ORDER BY 1,2;
