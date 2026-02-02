permit (
    principal,
    action == Action::"read_table",
    resource
)
when { principal in resource.readers };

permit (
    principal,
    action == Action::"write_table",
    resource
)
when { principal in resource.writers };
