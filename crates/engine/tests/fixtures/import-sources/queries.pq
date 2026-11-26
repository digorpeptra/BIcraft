section Section1;

shared Customers = let
    Source = #table(type table [Id = Int64.Type, Name = text], {{1, "Ada"}, {2, "Bob"}})
in
    Source;

shared Orders = let
    Source = #table(type table [Id = Int64.Type, Customer = Int64.Type], {{10, 1}, {11, 2}})
in
    Source;

shared Double = (x as number) => x * 2;

shared Threshold = 10 meta [IsParameterQuery = true, Type = "Number", IsParameterQueryRequired = true];
