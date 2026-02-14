# Money Transfer

Money transfer workflow entity demonstrating error handling and compensation.

## Example usage via CLI

Create a money transfer (spawns and ticks, blocks on withdraw extern):

    duralade entity spawn \
        --entity money_transfer.transfer \
        --id xfer1 \
        --in '{"from_account": "acc1", "to_account": "acc2", "amount": 100}'

Complete the withdraw extern and tick (blocks on deposit extern):

    duralade entity complete-extern \
        --id xfer1 \
        --invoke-num <num>

    duralade entity tick --id xfer1

Complete the deposit extern and tick (entity completes):

    duralade entity complete-extern \
        --id xfer1 \
        --invoke-num <num>

    duralade entity tick --id xfer1

Check entity completion status:

    duralade entity describe --id xfer1
