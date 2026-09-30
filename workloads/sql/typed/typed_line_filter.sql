SELECT DISTINCT l.orderkey, l.price, l.shipdate, l.comment
FROM Lineitem AS l CROSS JOIN Limits AS bounds
WHERE l.price >= bounds.minimum AND l.shipdate >= bounds.earliest
  AND l.linenumber > 1;
