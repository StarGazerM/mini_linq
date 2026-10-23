WITH
  Customer AS (SELECT DISTINCT c0 AS custkey FROM "mini-linq/raw/0"),
  Orders AS (SELECT DISTINCT c0 AS order_key, c1 AS custkey FROM "mini-linq/raw/1")
SELECT DISTINCT c.custkey FROM Customer c WHERE NOT EXISTS (SELECT 1 FROM Orders o WHERE o.custkey = c.custkey)
ORDER BY 1;
