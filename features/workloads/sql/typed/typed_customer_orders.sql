SELECT DISTINCT c.name, o.orderkey, o.orderdate, o.total
FROM Customer AS c JOIN Orders AS o USING (customerkey);
