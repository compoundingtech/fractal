//! The published attention list against the full attention computation: every window it
//! serves (every person's rows, each person's, and a person with none) equals
//! `client_attention_resources_at` at the publication's cut and clock, through asks, answers,
//! custom requests, replication in either order, rollback, reopen, checkpoint and projection
//! changes; and claims attention never reads publish the same rows without a fold.


    

    

