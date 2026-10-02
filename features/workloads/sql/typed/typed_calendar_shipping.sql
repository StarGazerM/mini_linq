SELECT DISTINCT c.label, s.orderkey, s.price, s.shipdate
FROM Calendar AS c JOIN Shipment AS s USING (shipdate);
