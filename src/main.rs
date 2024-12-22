use std::fs::File;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{JoinHandle, spawn};

use minimp3_fixed::{Decoder, Error, Frame};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{
    FftFixedInOut, Resampler
};

// MP3

fn load_and_play_mp3(path: &str) {

    // Open the path with an MP3 decoder.
    let mut decoder = Decoder::new(File::open(path).unwrap());

    // Read the frames, parse metadata, and de-interlace channel sample data.
    // Convert the sample data from i16 to f64 by taking fractions of i16::MAX
    let mut channel_data: Vec<Vec<f64>> = vec![];
    let mut current_sample_rate: i32 = 0;
    let mut current_channels: usize = 0;
    let mut samples_per_channel = 0;

    // Parse the first frame.
    match decoder.next_frame() {
        Ok(Frame {
               data,
               sample_rate,
               channels,
               layer,
               bitrate,
           }) => {
            current_sample_rate = sample_rate;
            current_channels = channels;
            for _ in 0..current_channels {
                channel_data.push(vec![]);
            }
            for i in 0..channel_data.len() {
                channel_data[i % current_channels].push(data[i] as f64 / (i16::MAX as f64));
            }
            samples_per_channel += channel_data.len() / current_channels;
        }
        Err(Error::Eof) => panic!("MP3 loaded at {} is empty.", path),
        Err(e) => panic!("{:?}", e),
    }

    // Read the remaining frames.
    loop {
        match decoder.next_frame() {
            Ok(Frame {
                   data,
                   sample_rate,
                   channels,
                   layer,
                   bitrate,
               }) => {
                println!("Got a frame. {} samples.", data.len());
                if sample_rate != current_sample_rate {
                    panic!("Sample rate cannot change across frames.");
                }
                if channels != current_channels {
                    panic!("Channel count cannot change across frames.")
                }
                for i in 0..data.len() {
                    channel_data[i % current_channels].push(data[i] as f64 / (i16::MAX as f64));
                }
                samples_per_channel += data.len() / current_channels;
                println!("Sample per channel is now {}", samples_per_channel);
            }
            Err(Error::Eof) => break,
            Err(e) => panic!("{:?}", e),
        }
    }

    // Get the target device configuration and determine the sample rate.
    // Use CPAL.

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no output device available");
    println!("Device! {}", device.name().unwrap());

    let supported_config = device.default_output_config().unwrap();
    let target_sample_rate = supported_config.sample_rate().0 as i32;

    // Use Rubato to resample the channel data.
    let mut resampler = FftFixedInOut::<f64>::new(
        current_sample_rate as usize,
        target_sample_rate as usize,
        1024,
        current_channels,
    ).unwrap();
    let mut input_buffer = resampler.input_buffer_allocate(true);
    let mut output_buffer = resampler.output_buffer_allocate(true);

    let mut output = vec!{};

    println!("Resampling.");
    let mut current_offset = 0;
    loop {
        println!("Looping resampling. {} samples left", channel_data[0].len());
        let chunk_size = resampler.input_frames_next();
        if chunk_size > channel_data[0].len() {
            break;
        }
        for i in 0..current_channels {
            input_buffer[i].copy_from_slice(&channel_data[i].drain(..chunk_size).as_slice());
            println!("Added {} samples to channel {}.", input_buffer[i].len(), i);
        }
        resampler.process_into_buffer(&input_buffer, &mut output_buffer, None).unwrap();

        for f in 0..resampler.output_frames_next() {
            for c in 0..current_channels {
                output.push(output_buffer[c][f] as f32);
            }
        }
    }

    if channel_data[0].len() > 0 {
        // Process the last samples in the stream.
        for i in 0..current_channels {
            input_buffer[i].clear();
            input_buffer[i].extend_from_slice(channel_data[i].drain(..).as_slice());
            println!("Added {} samples to channel {}.", input_buffer[i].len(), i);
        }
        resampler.process_partial_into_buffer(Some(&input_buffer), &mut output_buffer, None).unwrap();
        for f in 0..resampler.output_frames_next() {
            for c in 0..current_channels {
                output.push(output_buffer[c][f] as f32);
            }
        }
    }

    let finished = Arc::new(Mutex::new(false));
    let finished2 = finished.clone();
    let finished_condition = Arc::new(Condvar::new());
    let finished_condition2 = finished_condition.clone();

    println!("Building stream.");

    let stream = device.build_output_stream(
        &supported_config.into(),
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            println!("In audio sample callback. {:?}", info);
            let output_chunk_size = data.len();
            if output_chunk_size % current_channels != 0 {
                panic!("Weird output chunk size {}", output_chunk_size);
            }
            if output.len() < output_chunk_size {
                output.clear();
                *finished.lock().unwrap() = true;
                finished_condition.notify_all();
                return;
            }
            data.copy_from_slice(output.drain(0..output_chunk_size).as_slice());
        },
        move |err| {
            panic!("Error in audio stream: {}", err);
        },
        None
    ).unwrap();

    let foo = stream.play().unwrap();

    loop {
        let _ = finished_condition2.wait_while(finished2.lock().unwrap(), |fini: &mut bool| {
            return !*fini;
        }).expect("Something went wrong evaluating the conditional finished variable.");
        println!("Finished!");
        break;
    }
}

fn main() {
    println!("Hello, world!");

    let h : JoinHandle<()> = spawn(|| load_and_play_mp3("assets/Sounds/Music/006- Earthbound - Choose a File.mp3"));

    h.join().expect("Failed to join thread.");
}
