use std::f32::consts::PI;

use hardware::lidar::{Scan, ScanPoint};
use xilem::WidgetView;
use xilem::masonry::{
    core::MutateCtx,
    imaging::record::Scene,
    imaging::Painter,
    kurbo::{Circle, Stroke, Rect, Point, Size},
    palette,
    peniko::Color,
};
use xilem::style::Style;
use xilem::view::*;

pub struct LidarView {
    /// Distance in real-world meters the closes edge of the viewport should have to the robot
    min_viewport_distance: f64,

    /// Size of the robot in real-world meters
    robot_box: Size,

    raw_data: Scan,
}

impl LidarView {
    const POINT_DIAMETER: f64 = 1.0;
    const POINT_COLOR: Color = palette::css::RED;
    const ROBOT_COLOR: Color = palette::css::GREEN;

    pub fn new() -> Self {
        let starting_point_count = 800;
        let points: Vec<_> = std::iter::successors(Some(0.001), |prev| Some(prev + 0.005))
            .enumerate()
            .map(|(i, r)| ScanPoint::new(r, i as f32 * 2.0 * PI / starting_point_count as f32))
            .take(starting_point_count)
            .collect();
        Self {
            min_viewport_distance: 5.0,
            robot_box: Size {
                height: 0.25387,
                width: 0.1425,
            },
            raw_data: Scan {
                discarded_count: 0,
                points,
            },
        }
    }

    pub fn give_data(&mut self, scan: Scan) {
        self.raw_data = scan;
    }

    pub fn display(&mut self) -> impl WidgetView<Self> + use<> {
        let radius_control = flex_row((
            label(format!("Edge radius: {} meters", self.min_viewport_distance)),
            text_button("-", |lidar: &mut Self| lidar.min_viewport_distance -= 1.0),
            text_button("+", |lidar: &mut Self| lidar.min_viewport_distance += 1.0),
        )).cross_axis_alignment(CrossAxisAlignment::Start);
        let debug_info = label(format!("{} Points\n{} Discarded",
                self.raw_data.points.len(),
                self.raw_data.discarded_count))
            .color(palette::css::WHITE);
        let overlay_ui = flex_row((
            radius_control,
            FlexSpacer::Flex(2.0),
            debug_info,
        )).cross_axis_alignment(CrossAxisAlignment::Start);
        zstack::<_, (), _>((
            canvas(Self::canvas_update),
            overlay_ui,
        )).alignment(xilem::masonry::layout::UnitPoint::TOP_LEFT)
        // fork(
        //     main_view,
        //     self.scanning.then(|| task(
        //         Self::scan_task,
        //         |lidar: &mut Self, scan| {
        //             match scan {
        //                 Ok(scan) => lidar.raw_data = scan,
        //                 Err(e) => {
        //                     lidar.scanning = false;
        //                 },
        //             }
        //         }
        //     ))
        // )
    }

    // fn scan_task(proxy: MessageProxy<Result<Scan, ()>>, lidar: &mut Self) -> impl Future<Output = ()> + use <> {
    //     let dev_path = lidar.dev_path.clone();
    //     let start_time = lidar.start_time_mono;
    //     let exit_signal = Arc::new(AtomicBool::new(false));
    //     async move {
    //         let _guard = {
    //             let exit_signal = Arc::clone(&exit_signal);
    //             scopeguard::guard(exit_signal, |exit_signal| {
    //                 exit_signal.store(true, Ordering::Relaxed);
    //             })
    //         };
    //         let res = tokio::task::spawn_blocking(move || {
    //             
    //             let mut rplidar = match Self::setup_lidar(dev_path.as_str()) {
    //                 Ok(lidar) => lidar,
    //                 Err(banner) => {
    //                     let _ = proxy.message(Err(banner));
    //                     return
    //                 }
    //             };
    //
    //             log::trace!("Starting Lidar scanning");
    //             while !exit_signal.load(Ordering::Relaxed) {
    //                 match rplidar.grab_scan() {
    //                     Ok(scan) => {
    //                         let count = valid_scan.len();
    //                         log::debug!("Grabbed scan with {count} valid points");
    //
    //                         let scan = Scan {
    //                             timestamp: Instant::now() - start_time,
    //                             points: valid_scan,
    //                         };
    //                         if proxy.message(Ok(scan)).is_err() {
    //                             log::warn!("Couldn't send scan to main UI task");
    //                             break
    //                         }
    //                     }
    //                     Err(e) => {
    //                         log::warn!("Failed to grab LIDAR scan: {e}");
    //                     }
    //                 }
    //             }
    //         }).await;
    //         if let Err(e) = res {
    //             log::error!("Failed to end scan task: {e}");
    //         }
    //     }
    // }

    fn canvas_update(&mut self, _ctx: &mut MutateCtx<'_>, scene: &mut Scene, size: Size) {
        let mut painter = Painter::new(scene);
        painter.fill(
            Rect::new(0.0, 0.0, size.width, size.height),
            palette::css::BLACK,
        ).draw();
        let pixels_per_meter = size.width.min(size.height) / self.min_viewport_distance;
        let center = (size / 2.0).to_vec2();

        let robot_size = self.robot_box * pixels_per_meter;
        let top_left = center - (robot_size / 2.0).to_vec2();
        painter.stroke(
            Rect::from_origin_size(top_left.to_point(), robot_size),
            &Stroke::new(2.0),
            Self::ROBOT_COLOR,
        ).draw();

        for p in &self.raw_data.points {
            // Calculate center in cartesian coordinates. We use sin for x and cos for y
            // because theta is the clockwise angle relative to the y axis.
            let theta = p.radians();
            let x = center.x + (theta.sin() * p.meters()) as f64 * pixels_per_meter;
            let y = center.y - (theta.cos() * p.meters()) as f64 * pixels_per_meter;
            painter.fill(
                Circle {
                    radius: Self::POINT_DIAMETER / 2.0,
                    center: Point { x, y }
                },
                Self::POINT_COLOR,
            ).draw();
        }
    }
}

// fn format_duration(duration: Duration) -> String {
//     const HOURS_DIVISOR: u64 = 60 * 60;
//     const MINUTES_DIVISOR: u64 = 60;
//     let secs = duration.as_secs();
//     let hours = secs / HOURS_DIVISOR;
//     let secs = secs % HOURS_DIVISOR;
//     let minutes = secs / MINUTES_DIVISOR;
//     let secs = secs % MINUTES_DIVISOR;
//     let millis = duration.subsec_millis();
//
//     format!("{hours:02}:{minutes:02}:{secs:02}.{millis:03}")
// }
