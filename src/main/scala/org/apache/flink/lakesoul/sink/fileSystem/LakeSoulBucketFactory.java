package org.apache.flink.lakesoul.sink.fileSystem;

import org.apache.flink.core.fs.Path;

import org.apache.flink.lakesoul.sink.LakeSoulRollingPolicyImpl;
import org.apache.flink.lakesoul.sink.LakesoulTableSink;
import org.apache.flink.streaming.api.functions.sink.filesystem.BucketWriter;
import org.apache.flink.streaming.api.functions.sink.filesystem.FileLifeCycleListener;
import org.apache.flink.streaming.api.functions.sink.filesystem.OutputFileConfig;

import javax.annotation.Nullable;
import java.io.IOException;
import java.io.Serializable;

public interface LakeSoulBucketFactory <IN, BucketID> extends Serializable {
    LakeSoulBucket<IN, BucketID> getNewBucket(int var1,
                                      BucketID var2,
                                      Path var3, long var4,
                                      BucketWriter<IN, BucketID> var6,
                                              LakeSoulRollingPolicyImpl<IN, BucketID> var7,
                                      @Nullable FileLifeCycleListener<BucketID> var8,
                                      OutputFileConfig var9,
                                              String var10
    ) throws IOException;

    LakeSoulBucket<IN, BucketID> restoreBucket(int var1, long var2,
                                       BucketWriter<IN, BucketID> var4,
                                               LakeSoulRollingPolicyImpl<IN, BucketID> var5,
                                       BucketState<BucketID> var6,
                                       @Nullable FileLifeCycleListener<BucketID> var7,
                                       OutputFileConfig var8, String var19
    ) throws IOException;
}
