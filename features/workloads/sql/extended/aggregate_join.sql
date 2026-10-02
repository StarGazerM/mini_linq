WITH
  Sale AS (SELECT DISTINCT c0,c1,c2 FROM "mini-linq/raw/0"),
  Allowed AS (SELECT DISTINCT c0 FROM "mini-linq/raw/1")
SELECT DISTINCT * FROM (
SELECT Sale.c0,CAST(sum(Sale.c2) AS INTEGER),min(Sale.c2) FROM Sale JOIN Allowed ON Sale.c0=Allowed.c0 GROUP BY Sale.c0
) AS result
ORDER BY 1,2,3;
