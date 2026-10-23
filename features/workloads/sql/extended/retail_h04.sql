WITH
  Orders AS (SELECT DISTINCT c0 AS order_key FROM "mini-linq/raw/0"),
  LateLineitem AS (SELECT DISTINCT c0 AS order_key FROM "mini-linq/raw/1")
SELECT DISTINCT o.order_key FROM Orders o JOIN LateLineitem l ON l.order_key = o.order_key
ORDER BY 1;
