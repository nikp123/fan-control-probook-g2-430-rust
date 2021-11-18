#![allow(unused)]

use std::io::{self,BufReader, BufRead, Read, Seek};
use std::fs::{self,File};
use std::{env,ptr,path,thread,time};

// C things
use std::ffi::CString;

use std::io::{Result, Error, ErrorKind};
//use color_eyre::Report;

struct PortCommunicator {
    file: i32,
}

impl PortCommunicator {
    // public
    pub fn new() -> Result<PortCommunicator> {
        let filename = CString::new("/dev/port")?;
        let file;
        unsafe {
            file = libc::open(filename.into_raw(), libc::O_RDWR);
            libc::lseek(file, 0, libc::SEEK_CUR);
        }

        if file == -1 {
            return Err(Error::new(ErrorKind::Other, "Failed to open port file"));
        }

        Ok(PortCommunicator {
            file
        })
    }

    pub fn write(&mut self, value: u8, port: u8) -> Result<()> {
        let ret;
        unsafe {
            if libc::lseek(self.file, port as libc::off_t, libc::SEEK_SET) == -1 {
                return Err(Error::new(io::ErrorKind::UnexpectedEof, "lseek failed"));
            }
            let ptr: *const libc::c_void = ptr::addr_of!(value) as *const libc::c_void;
            let error = libc::write(self.file, ptr, 1);
            match error {
                -1 => ret = Err(Error::new(ErrorKind::Other, "read failed")),
                _ => ret = Ok(())
            }
        }
        ret
    }

    pub fn read(&mut self, port: u8) -> Result<u8> {
        let value: u8 = 0;
        let ret;
        unsafe {
            if libc::lseek(self.file, port as libc::off_t, libc::SEEK_SET) == -1 {
                return Err(Error::new(ErrorKind::UnexpectedEof, "lseek failed"))
            }
            let ptr: *mut libc::c_void = ptr::addr_of!(value) as *mut libc::c_void;
            let error = libc::read(self.file, ptr, 1);
            match error {
                -1 => ret = Err(Error::new(ErrorKind::Other, "read failed")),
                0 => ret = Err(Error::new(ErrorKind::UnexpectedEof, "read failed")),
                _ => ret = Ok(value)
            }
        }
        ret
    }

    fn wait_write_ec(&mut self) -> Result<()> {
        self.wait_write(0x66)
    }

    fn wait_read_ec(&mut self) -> Result<()> {
        self.wait_read(0x66)
    }

    pub fn write_ec(&mut self, port: u8, value: u8) -> Result<()> {
        self.wait_write_ec()?;
        self.write(0x81, 0x66)?;
        self.wait_write_ec()?;
        self.write(port, 0x62)?;
        self.wait_write_ec()?;
        self.write(value, 0x62)
    }

    pub fn read_ec(&mut self, port: u8) -> Result<u8> {
        self.wait_write_ec()?;
        self.write(0x80, 0x66)?;
        self.wait_write_ec()?;
        self.write(port, 0x62);
        self.wait_read_ec()?;
        self.read(0x62)
    }


    // private
    fn wait_write(&mut self, port: u8) -> Result<()> {
        let mut i = 0;
        let onehundreth = time::Duration::new(0, 10000000); 
        while (((self.read(port)?) & 0x02) > 0) && (i < 10000) {
            i = i + 1;
            thread::sleep(onehundreth);
        }

        if i < 10000 {
            return Ok(());
        }

        Err(Error::new(ErrorKind::TimedOut, "Waiting for write failed!"))
    }

    fn wait_read(&mut self, port: u8) -> Result<()> {
        let mut i = 0;
        let onehundreth = time::Duration::new(0, 10000000); 
        while (((self.read(port)?) & 0x01) > 0) && (i < 10000) {
            i = i + 1;
            thread::sleep(onehundreth);
        }

        if i < 10000 {
            return Ok(());
        }

        Err(Error::new(ErrorKind::TimedOut, "Waiting for read failed!"))
    }

    #[allow(dead_code)]
    fn drop(&mut self) -> Result<()> {
        let ret_val;
        unsafe {
            ret_val = libc::close(self.file);
        }
        if ret_val == 0 {
            return Ok(())
        }
        Err(Error::new(ErrorKind::Other, "Failed to close file"))
    }
}

struct CPUTemp {
    path: String
}

impl CPUTemp {
    pub fn new(device_name: &str) -> Result<CPUTemp> {
        let mut pathbuf;
        let path;
        pathbuf = path::PathBuf::from("/sys/devices/virtual/thermal/");
        pathbuf.push(device_name);
        pathbuf.push("temp");

        match pathbuf.to_str() {
            Some(some) => path = some.to_string(),
            None       => return Err(Error::new(ErrorKind::Other, "Failed to convert string"))
        }

        fs::File::open(&path)?;

        Ok(CPUTemp {
            path
        })
    }

    /**
     * Returned in degrees Celsius
     **/
    pub fn read(&self) -> Result<f32> {
        let mut file   = fs::File::open(self.path.to_string())?;
        let mut reader = BufReader::new(file);
        let mut line   = String::new();

        reader.read_line(&mut line)?;

        line = String::from(line.trim_end());

        let mut float = line.parse::<u32>().unwrap()
            as f32;

        float /= 1000.0;
        Ok(float)
    }
}

struct FanControllerConfig {
    temp_interval_count: u8,
    temp_interval_lenght: time::Duration,

    throttle: time::Duration,
    throttle_off: time::Duration,

    temp_fan_start_speed: u8,
    temp_fan_max_speed: u8,

    steps: u8,
}

impl Default for FanControllerConfig {
    fn default() -> FanControllerConfig {
        Self {
            temp_interval_count: 5,
            temp_interval_lenght: time::Duration::new(5, 0),

            throttle: time::Duration::new(30, 0),
            throttle_off: time::Duration::new(10, 0),

            temp_fan_start_speed: 60,
            temp_fan_max_speed: 80,

            steps: 8,
        }
    }
}

struct FanController {
    last_temp: u8,
    last_speed: u8,
    new_speedf: u8,
    temp_last_interval: time::SystemTime,
 
    config: FanControllerConfig,
    temp: CPUTemp,
    port: PortCommunicator,
}

impl FanController {
    pub fn new(temp: CPUTemp, port: PortCommunicator,
               config: Option<FanControllerConfig>) -> FanController {
        FanController {
            temp,
            port,
            config: config.unwrap_or(Default::default()),
            last_temp: 0,
            last_speed: 0,
            new_speedf: 0,
            temp_last_interval: time::SystemTime::now(),
        }
    }

    pub fn run(&self) -> Result<()> {
        let avg_temp = self.get_average_temperature()? as u8;

        if avg_temp < self.config.temp_fan_start_speed {
            if self.last_speed > 75 {
                if self.temp_last_interval.elapsed().unwrap()
                    > self.config.throttle {
                    self.new_speedf = 0xff;
                }
            } else if self.new_speedf == 0xff {
                self.new_speedf = 0xff
            } else {
                self.new_speedf = self.config.temp_interval_lenght.as_secs() as u8;
            }
        } else if avg_temp > self.config.temp_fan_max_speed {
            self.new_speedf = 0x0;
            self.config.temp_interval_lenght = time::Duration::new(10, 0);
        } else {
            if avg_temp < self.last_temp {
                if self.temp_last_interval.elapsed().unwrap()
                    < self.config.throttle_off {
                    self.new_speedf = self.new_fan_speed(avg_temp);
                    self.temp_last_interval = time::SystemTime::now();
                }
            } else {
                self.new_speedf = self.new_fan_speed(avg_temp);
                self.config.temp_interval_lenght = 
                    time::Duration::new(self.calculate_next_interval(avg_temp),
                    0);
                self.temp_last_interval = time::SystemTime::now();
            }
        }

        Ok(())
    }

    fn calculate_next_interval(&self, new_temp: u8) -> u8 {
        let next_interval = 0;
        let new_interval;

        for i in 0..self.config.steps {
            if next_interval < new_temp {
                next_interval = self.config.temp_fan_start_speed;
                next_interval += 
                    (self.config.temp_fan_max_speed-self.config.temp_fan_start_speed)
                    * i / self.config.steps;
                new_interval = 1 + self.config.steps - i;
            }
        }
        new_interval
    }

    fn new_fan_speed(&self, new_temp: u8) -> u8 {
        let new_speed: u8;
        self.last_speed = self.new_speedf;
        new_speed = (new_temp-self.config.temp_fan_max_speed) *
            (new_temp + self.config.temp_fan_max_speed);
        new_speed *= -24;
        new_speed /= 1000;
        new_speed
    }

    // returns in Celsius
    fn get_average_temperature(&self) -> Result<f32> {
        let num_intervals = self.config.temp_interval_count;
        let mut temps: Vec<f32>;

        for i in 0..self.config.temp_interval_count {
            temps.push(self.temp.read()?);
            thread::sleep(self.config.temp_interval_lenght);
        }

        let last_temp = temps.iter().cloned().fold(0./0., f32::max); 
        Ok(last_temp)
    }
}

fn main() -> Result<()> {
    let port = 0x2f;

    let mut fan = PortCommunicator::new()?;
    let temp = CPUTemp::new("thermal_zone6")?;

    let mut current_speed = 0;
    let a_second = time::Duration::new(1, 0);
    let quarter_second = time::Duration::new(0, 250000000);

    loop {
        let mut temps: [f32; 5] = [0.0; 5];
        let last_temp;

        {
            temps[0] = temp.read()?;
            thread::sleep(quarter_second);
            temps[1] = temp.read()?;
            thread::sleep(quarter_second);
            temps[2] = temp.read()?;
            thread::sleep(quarter_second);
            temps[3] = temp.read()?;
            thread::sleep(quarter_second);
            temps[4] = temp.read()?;
            thread::sleep(quarter_second);
        }
        last_temp = temps.iter().cloned().fold(0./0., f32::max); 

        let temp_min = 60.0;
        let temp_max = 85.0;

        if last_temp < temp_min {
            current_speed = 0;
        } else if last_temp < temp_max {
            let mut speed = (last_temp-temp_min)/(temp_max-temp_min);
            current_speed = (speed*128.0 + 127.0) as u8;
        } else {
            current_speed = 255;
        }

        // go full tilt
        //fan.write_ec(port, 0)?;
        //thread::sleep(quarter_second);

        println!("temp: {}, speed: {}", last_temp, current_speed);

        fan.write_ec(port, 0xff-current_speed)?;
        thread::sleep(a_second);
    }
}
