#[macro_use]
extern crate state_machine_future;

#[macro_use]
extern crate futures;

use futures::{Async, Future, Poll};
use state_machine_future::RentToOwn;

/// The result of a game.
pub struct GameResult {
    winner: Player,
    loser: Player,
}

/// Some kind of simple turn based game.
///
/// ```text
///              Invite
///                |
///                |
///                | accept invitation
///                |
///                |
///                V
///           WaitingForTurn --------+
///                |   ^             |
///                |   |             | receive turn
///                |   |             |
///                |   +-------------+
/// game concludes |
///                |
///                |
///                |
///                V
///            Finished
/// ```
#[derive(StateMachineFuture)]
enum Game {
    /// The game begins with an invitation to play from one player to another.
    ///
    /// Once the invited player accepts the invitation over HTTP, then we will
    /// switch states into playing the game, waiting to recieve each turn.
    #[state_machine_future(start, transitions(WaitingForTurn))]
    Invite {
        invitation: HttpInvitationFuture,
        from: Player,
        to: Player,
    },

    // We are waiting on a turn.
    //
    // Upon receiving it, if the game is now complete, then we go to the
    // `Finished` state. Otherwise, we give the other player a turn.
    #[state_machine_future(transitions(WaitingForTurn, Finished))]
    WaitingForTurn {
        turn: HttpTurnFuture,
        active: Player,
        idle: Player,
    },

    // The game is finished with a `GameResult`.
    //
    // The `GameResult` becomes the `Future::Item`.
    #[state_machine_future(ready)]
    Finished(GameResult),

    // Any state transition can implicitly go to this error state if we get an
    // `HttpError` while waiting on a turn or invitation acceptance.
    //
    // This `HttpError` is used as the `Future::Error`.
    #[state_machine_future(error)]
    Error(HttpError),
}

// Now, we implement the generated state transition polling trait for our state
// machine description type.

impl PollGame for Game {
    fn poll_invite<'a>(
        invite: &'a mut RentToOwn<'a, Invite>
    ) -> Poll<AfterInvite, HttpError> {
        // See if the invitation has been accepted. If not, this will early
        // return with `Ok(Async::NotReady)` or propagate any HTTP errors.
        try_ready!(invite.invitation.poll());

        // We're ready to transition into the `WaitingForTurn` state, so take
        // ownership of the `Invite` and then construct and return the new
        // state.
        let invite = invite.take();
        let waiting = WaitingForTurn {
            turn: invite.from.request_turn(),
            active: invite.from,
            idle: invite.to,
        };
        Ok(Async::Ready(waiting.into()))
    }

    fn poll_waiting_for_turn<'a>(
        waiting: &'a mut RentToOwn<'a, WaitingForTurn>
    ) -> Poll<AfterWaitingForTurn, HttpError> {
        // See if the next turn has arrived over HTTP. Again, this will early
        // return `Ok(Async::NotReady)` if the turn hasn't arrived yet, and
        // propagate any HTTP errors that we might encounter.
        let turn = try_ready!(waiting.turn.poll());

        // Ok, we have a new turn. Take ownership of the `WaitingForTurn` state,
        // process the turn and if the game is over, then transition to the
        // `Finished` state, otherwise swap which player we need a new turn from
        // and request the turn over HTTP.
        let waiting = waiting.take();
        if let Some(game_result) = process_turn(turn) {
            let finished = Finished(game_result);
            Ok(Async::Ready(finished.into()))
        } else {
            let next_waiting = WaitingForTurn {
                turn: waiting.idle.request_turn(),
                active: waiting.idle,
                idle: waiting.active,
            };
            Ok(Async::Ready(next_waiting.into()))
        }
    }
}

// To spawn a new `Game` as a `Future` on whatever executor we're using (for
// example `tokio`), we use `Game::start` to construct the `Future` in its start
// state and then pass it to the executor.
fn spawn_game(handle: TokioHandle) {
    let from = get_some_player();
    let to = get_another_player();
    let invitation = invite(&from, &to);
    let future = Game::start(invitation, from, to);
    handle.spawn(future)
}
