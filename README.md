# teral
This was my cryptocurrency project. It is very bad and most of it, if worked in the past, was ruined. It is unmaintained and will be rewritten with libp2p in the future.

before I start, all communication is through the gossip network.
the consensus is comprised of epochs, which I decided to be a day, and slots which are 20 seconds each. we select a fixed member-count committee after every epoch from the stake distribution. every leader selection, we select the leader according to, again, the stake distribution.
note, that the seed in the committee selection could be something like the last block's hash, which is not known ahead of time, or the committee member's pubkey hashes, that _is_ known.

the committee members should vote on the first block they see. if they did not see a block in those 20 seconds, they generate a new leader with the seed on the leader's pubkey hash. again, if they do see the block from the previous leader before the new leader and it is valid, they will vote for it (it has been some time since I thought about this, I might be wrong and this is insecure, send an email / open an issue, it would be welcomed). 
Until this becomes a non-time-dependent protocol, every epoch will have a time synchronization session and the committee acts as leaders (they have stake, we can trust them) (just an idea, we can also choose a committee member only if it participated in a time synchronization? Im not sure. Or firstly the committee does it themselves?)
