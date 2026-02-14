# Set

Set data structure for storing unique values.

## Example usage via CLI

Create a set with initial items:

    duralade entity spawn \
        --entity set.set \
        --id set1 \
        --in '{"initial_items": ["a", "b", "c"]}'

Add an item:

    duralade entity invoke-func-noblock \
        --id set1 \
        --func add \
        --in '{"item": "d"}'

Check if item exists:

    duralade entity view \
        --id set1 \
        --func contains \
        --in '{"item": "b"}'

Remove an item:

    duralade entity invoke-func-noblock \
        --id set1 \
        --func remove \
        --in '{"item": "a"}'

Get current length:

    duralade entity view \
        --id set1 \
        --func len
