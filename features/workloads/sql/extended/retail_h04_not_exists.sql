WITH
  Orders AS (SELECT DISTINCT c0 AS order_key FROM "mini-linq/raw/0"),
  LateLineitem AS (SELECT DISTINCT c0 AS order_key FROM "mini-linq/raw/1")
SELECT DISTINCT o.order_key FROM Orders o WHERE NOT EXISTS (SELECT 1 FROM LateLineitem l WHERE l.order_key = o.order_key)
ORDER BY 1;
