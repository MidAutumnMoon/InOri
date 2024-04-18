use std::{
    thread,
    sync::mpsc,
};

use tracing::debug;

use itertools::Itertools;

use crate::asset::{
    Asset,
    DecryptAsset
};



pub enum TaskStatus {
    Done,
    Fail( anyhow::Error ),
}

pub struct TaskInfo {
    asset: Asset,
    status: TaskStatus,
}


#[ tracing::instrument( skip_all ) ]
pub fn submit_assets(
    assets: Vec<Asset>,
    threads: usize,
) {

    debug!( "process {} assets with {} threads",
        assets.len(), threads
    );

    if assets.is_empty() {
        use colored::Colorize;
        println!( "{}", "No assets to process".blue() );
        return
    }


    let total_tasks = assets.len();

    let asset_chunks = {
        let total = assets.len();
        let chunks = assets.into_iter()
            .chunks( total.div_ceil( threads ) );
        chunks.into_iter()
            .map( |ck| ck.collect_vec() )
            .collect_vec()
    };

    let ( og_sender, receiver ) =
        mpsc::channel::<TaskInfo>();


    thread::scope( |scope| {

        for chunk in asset_chunks {
            let sender = og_sender.clone();
            scope.spawn( || many_assets( chunk, sender ) );
        }

        drop( og_sender );

        scope.spawn(
            || display_taskinfo( total_tasks, receiver )
        );

    } );

}


#[ tracing::instrument(
    skip_all,
    fields( count = assets.len() )
) ]
fn many_assets(
    assets: Vec<Asset>,
    sender: mpsc::Sender<TaskInfo>
) {
    debug!(
        "process assets of count {}",
        assets.len()
    );

    for one in assets {
        let status = one_asset( one.clone() );
        sender.send( TaskInfo {
            asset: one,
            status
        } ).unwrap();
    }
}

#[ tracing::instrument ]
fn one_asset( asset: Asset ) -> TaskStatus {
    debug!( "process one asset" );

    let da = match DecryptAsset::new( asset ) {
        Ok( dec ) => dec,
        Err( e ) => return TaskStatus::Fail( e )
    };

    match da.write_decrypted() {
        Ok( _ ) => TaskStatus::Done,
        Err( e ) => TaskStatus::Fail( e )
    }
}


#[ tracing::instrument( skip_all ) ]
fn display_taskinfo(
    total_tasks: usize,
    receiver: mpsc::Receiver<TaskInfo>
) {
    debug!( "read and display tasks' status" );

    for ( count, info ) in
        receiver.iter().enumerate()
    {
        use colored::Colorize;
        use std::io::prelude::*;

        use TaskInfo as I;
        use TaskStatus as S;

        let I { status, asset } = info;

        let path = asset.origin.display();

        let msg = match status {
            S::Done =>
                format!( "(ok) {path}" ).blue(),
            S::Fail( e ) =>
                format!( "(err) {path} {e:?}" ).red(),
        };

        let mut stdout = std::io::stdout().lock();

        writeln!{ stdout, "{}/{} {msg}",
            count + 1,
            total_tasks,
        }.unwrap();
    }
}
