# Volley Shot

Concurrent extern calls demonstrating task spawning and concurrency control.

## Example usage via CLI

Fire a volley at multiple points:

    duralade entity spawn \
        --entity volley_shot.volley_shot \
        --id volley1 \
        --in '{"points": [{"x": 5, "y": 3}, {"x": 7, "y": 8}, {"x": 2, "y": 4}]}'

Tick starts all shoot externs concurrently:

    duralade entity tick --id volley1

Complete each shoot extern (can be done in any order):

    duralade entity complete-extern \
        --id volley1 \
        --invoke-num <event_num_1> \
        --result '{"hit": false}'

    duralade entity complete-extern \
        --id volley1 \
        --invoke-num <event_num_2> \
        --result '{"hit": true}'

Tick processes results - returns true as soon as any hit:

    duralade entity tick --id volley1

Check final result:

    duralade entity describe --id volley1
